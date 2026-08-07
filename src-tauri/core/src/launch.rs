//! Starting a game.
//!
//! **This is the only place in the app that spawns a process**, and everything
//! about it is shaped by that.
//!
//! WHY IT IS SAFE TO HAVE AT ALL
//! ----------------------------
//! The plugin model's rule is *a plugin is data, never code* — no shell, no
//! `exec` step, no environment read. A launcher obviously has to run something,
//! so the question is what a hostile or careless launch file could achieve. The
//! answer is bounded to exactly this:
//!
//!   * **The executable is a path relative to the game directory**, resolved
//!     through the same [`crate::plugins::sandbox`] that installers use. The
//!     type cannot express an absolute path, so a launch file cannot name
//!     `/bin/sh` — the worst it can do is run a binary that is already inside a
//!     folder the user pointed us at and chose to install a game into.
//!   * **Or a URI with a game-client scheme** (`steam://…`), handed to the OS
//!     opener. Checked against a closed scheme list, with control characters
//!     and quotes refused.
//!   * **There is no shell.** Arguments are an argv vector passed to
//!     `Command::args`. `;`, backticks, `$(…)`, `&&` and newlines are inert
//!     bytes to a process that was never handed to `sh -c`.
//!   * **The environment is a whitelist of what the file sets**, layered on the
//!     inherited environment. It cannot READ one.
//!
//! WHAT THE USER STILL DECIDES
//! --------------------------
//! [`LaunchPlan`] is produced and returned WITHOUT running anything, so the app
//! can show the resolved command line and let the user approve it. That is not
//! decoration: "here is exactly what is about to run" is the only thing that
//! makes a declarative launcher auditable by the person it affects.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::plugins::apps::{AppPluginFile, LaunchSpec};
use crate::plugins::manifest::{FsRoot, PathRef};
use crate::plugins::sandbox::Sandbox;

/// The declarative options an install carries, as the launcher sees them.
///
/// Mirrors `InstallOptionsSchema` in the API contract. Every field optional,
/// every one meaning "leave it alone" when absent — a launcher that silently
/// forces a resolution on somebody who never asked is worse than one with no
/// settings at all.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LaunchOptions {
    pub graphics: Option<String>,
    pub window_mode: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub monitor: Option<u32>,
    pub fps_limit: Option<u32>,
    pub vsync: Option<bool>,
    pub memory_mb: Option<u32>,
    pub hide_on_launch: Option<bool>,
    pub confirm_updates: Option<bool>,
}

impl LaunchOptions {
    /// The option table as `{placeholder}` values.
    ///
    /// A boolean becomes `1`/`0` rather than `true`/`false` — every game that
    /// takes one on a command line takes the numeric form, and a launch file
    /// wanting the word can write it literally.
    fn placeholders(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();

        let mut put = |key: &str, value: String| {
            out.insert(key.to_string(), value);
        };

        if let Some(v) = &self.graphics {
            put("graphics", v.clone());
        }
        if let Some(v) = &self.window_mode {
            put("windowMode", v.clone());
        }
        if let Some(v) = self.width {
            put("width", v.to_string());
        }
        if let Some(v) = self.height {
            put("height", v.to_string());
        }
        if let Some(v) = self.monitor {
            put("monitor", v.to_string());
        }
        if let Some(v) = self.fps_limit {
            put("fpsLimit", v.to_string());
        }
        if let Some(v) = self.vsync {
            put("vsync", if v { "1".into() } else { "0".into() });
        }
        if let Some(v) = self.memory_mb {
            put("memoryMb", v.to_string());
        }

        out
    }

    /// Which `optionArgs` keys are set. A key that is absent contributes
    /// nothing, which is how "leave it alone" is expressed.
    fn active_keys(&self) -> Vec<&'static str> {
        let mut keys = Vec::new();

        if self.graphics.is_some() {
            keys.push("graphics");
        }
        if self.window_mode.is_some() {
            keys.push("windowMode");
        }
        if self.width.is_some() {
            keys.push("width");
        }
        if self.height.is_some() {
            keys.push("height");
        }
        if self.monitor.is_some() {
            keys.push("monitor");
        }
        if self.fps_limit.is_some() {
            keys.push("fpsLimit");
        }
        // Only when ON: a game's `--vsync` flag has no "off" spelling, and the
        // launch file can declare `vsyncOff` if it needs one.
        if self.vsync == Some(true) {
            keys.push("vsync");
        }
        if self.memory_mb.is_some() {
            keys.push("memoryMb");
        }

        keys
    }
}

/// Everything the placeholders can be filled from, besides the options.
#[derive(Debug, Clone, Default)]
pub struct LaunchContext {
    pub install_name: Option<String>,
    pub game_version: Option<String>,
    pub loader: Option<String>,
    /// The install's own directory, when it has one. Available as `{installDir}`
    /// so a game that takes `--gameDir` can be pointed at a profile.
    pub install_dir: Option<PathBuf>,
    /// The game directory itself, as `{gameDir}`.
    pub game_dir: Option<PathBuf>,
}

/// What is about to run. Produced without side effects, shown to the user,
/// then executed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchPlan {
    /// The rule that produced this, e.g. `minecraft/launch.json`.
    pub rule: String,
    /// Absolute path to the executable, when this is a process launch.
    pub program: Option<String>,
    /// The URI, when this is a client hand-off. Exactly one of the two is set.
    pub uri: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// Environment overrides only. The child also inherits ours.
    pub env: BTreeMap<String, String>,
    /// Hide the app's window while the game runs, from the install's options.
    pub hide_window: bool,
}

/// Fill `{key}` occurrences from a table.
///
/// A literal replacement, never an expression. An unresolved placeholder is
/// reported by [`has_unresolved`] rather than passed through, because a game
/// receiving a literal `--width {width}` will either refuse to start or start
/// wrong, and both are worse than the argument being dropped.
fn fill(template: &str, table: &BTreeMap<String, String>) -> String {
    let mut out = template.to_string();

    for (key, value) in table {
        out = out.replace(&format!("{{{key}}}"), value);
    }

    out
}

/// Does a filled string still contain a `{…}` placeholder?
fn has_unresolved(value: &str) -> bool {
    let Some(open) = value.find('{') else {
        return false;
    };

    value[open..].contains('}')
}

/// Executable name for this platform, from the spec's per-platform table.
///
/// One game is one directory but three binaries, and a launch file that had to
/// pick one at authoring time would only ever work on the author's OS.
fn exec_for_platform(spec: &LaunchSpec) -> Option<&String> {
    let key = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };

    spec.exec_platform.get(key).or(spec.exec.as_ref())
}

/// Build the plan.
///
/// `sandbox` must be one built for this rule with the game directory as
/// `gameDir` — the executable and the working directory are both resolved
/// through it, which is what stops a launch file naming a path outside the
/// folder the user chose.
pub fn plan(
    rule: &AppPluginFile,
    sandbox: &Sandbox,
    options: &LaunchOptions,
    ctx: &LaunchContext,
) -> AppResult<LaunchPlan> {
    let spec = rule
        .launch
        .as_ref()
        .ok_or_else(|| AppError::invalid("That rule is not a launch spec."))?;

    let mut table = options.placeholders();

    if let Some(v) = &ctx.install_name {
        table.insert("installName".into(), v.clone());
    }
    if let Some(v) = &ctx.game_version {
        table.insert("gameVersion".into(), v.clone());
    }
    if let Some(v) = &ctx.loader {
        table.insert("loader".into(), v.clone());
    }
    if let Some(v) = &ctx.install_dir {
        table.insert("installDir".into(), v.display().to_string());
    }
    if let Some(v) = &ctx.game_dir {
        table.insert("gameDir".into(), v.display().to_string());
    }

    // ------------------------------------------------------------- Target
    let (program, uri) = match (&spec.uri, exec_for_platform(spec)) {
        (Some(uri), _) => {
            let filled = fill(uri, &table);

            if has_unresolved(&filled) {
                return Err(AppError::invalid(
                    "The launch URI has placeholders this install cannot fill.",
                ));
            }

            (None, Some(filled))
        }
        (None, Some(exec)) => {
            /*
             * Through the sandbox, with `write: false`. A launch spec needs no
             * write grant, and asking for one would be the difference between
             * "run something in the game folder" and "put something there
             * first".
             */
            let resolved = sandbox.resolve(
                &PathRef {
                    root: FsRoot::GameDir,
                    path: fill(exec, &table),
                },
                false,
            )?;

            if !resolved.is_file() {
                return Err(AppError::invalid(format!(
                    "The game's launcher is not where the rule expects it: {}",
                    resolved.display()
                )));
            }

            (Some(resolved), None)
        }
        (None, None) => {
            return Err(AppError::invalid(
                "That launch rule names nothing to start.",
            ))
        }
    };

    // --------------------------------------------------------------- Args
    let mut args: Vec<String> = Vec::new();

    for template in &spec.args {
        let filled = fill(template, &table);

        /*
         * An argument that still holds a placeholder is DROPPED rather than
         * passed through. That is what makes `--width {width}` correct in a
         * launch file: with no width configured the argument disappears, and
         * the game uses its own setting. Passing the literal text through
         * would either be rejected by the game or, worse, parsed as a filename.
         */
        if has_unresolved(&filled) {
            continue;
        }

        args.push(filled);
    }

    // Option-contributed arguments, in the order the option keys are listed so
    // a launch file's output is deterministic.
    for key in options.active_keys() {
        let Some(extra) = spec.option_args.get(key) else {
            continue;
        };

        for template in extra {
            let filled = fill(template, &table);

            if has_unresolved(&filled) {
                continue;
            }

            args.push(filled);
        }
    }

    if args.len() > 128 {
        return Err(AppError::invalid(
            "That launch produces too many arguments.",
        ));
    }

    // Belt and braces on top of the schema: a NUL in an argument is an error on
    // every platform's spawn, and a newline is how one argument becomes two in
    // anything that later logs and replays this.
    for arg in &args {
        if arg.contains('\0') || arg.contains('\n') || arg.contains('\r') {
            return Err(AppError::sandbox(
                "A launch argument contains a control character.",
            ));
        }
    }

    // ----------------------------------------------------------------- Cwd
    let cwd = match &spec.cwd {
        Some(rel) => Some(sandbox.resolve(
            &PathRef {
                root: FsRoot::GameDir,
                path: fill(rel, &table),
            },
            false,
        )?),
        // Default to the game directory itself, which is what nearly every
        // game expects and what a relative asset path inside one resolves
        // against.
        None => sandbox.root_path(FsRoot::GameDir).map(Path::to_path_buf),
    };

    // ----------------------------------------------------------------- Env
    let mut env = BTreeMap::new();

    for (key, template) in &spec.env {
        let value = fill(template, &table);

        if has_unresolved(&value) {
            continue;
        }

        if key.contains('\0') || value.contains('\0') {
            return Err(AppError::sandbox(
                "A launch environment value contains a NUL byte.",
            ));
        }

        env.insert(key.clone(), value);
    }

    Ok(LaunchPlan {
        rule: rule.source.clone(),
        program: program.map(|p| p.display().to_string()),
        uri,
        args,
        cwd: cwd.map(|p| p.display().to_string()),
        env,
        hide_window: options.hide_on_launch.unwrap_or(false),
    })
}

/// A one-line rendering of the plan, for the confirmation dialog and the log.
///
/// Quoted for READABILITY only. Nothing anywhere consumes this string — the
/// real launch uses the argv vector — and it must never be fed back into a
/// shell, which is why it is a separate function rather than the plan's
/// `Display`.
pub fn describe(plan: &LaunchPlan) -> String {
    if let Some(uri) = &plan.uri {
        return uri.clone();
    }

    let mut out = plan.program.clone().unwrap_or_default();

    for arg in &plan.args {
        out.push(' ');

        if arg.contains(' ') {
            out.push('"');
            out.push_str(arg);
            out.push('"');
        } else {
            out.push_str(arg);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::apps::AppPlugins;
    use crate::plugins::manifest::{FsGrant, Manifest, Permissions};
    use std::collections::HashMap;

    fn sandbox_over(game: &Path) -> Sandbox {
        let manifest = Manifest {
            manifest_version: 1,
            id: "app.test".into(),
            name: "t".into(),
            version: "1".into(),
            author: "t".into(),
            description: None,
            homepage: None,
            apps: vec![],
            permissions: Permissions {
                fs: vec![FsGrant {
                    root: FsRoot::GameDir,
                    path: String::new(),
                    write: false,
                }],
                ..Default::default()
            },
            installer: None,
            server_query: None,
            theme: None,
        };

        let mut available: HashMap<&'static str, PathBuf> = HashMap::new();
        available.insert("gameDir", game.to_path_buf());

        Sandbox::build(&manifest, &available).expect("sandbox")
    }

    fn rule_from(body: &str) -> AppPluginFile {
        let tmp = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
        let dir = tmp.path().join("app/minecraft");

        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("launch.json"), body).expect("write");

        let plugins = AppPlugins::load(tmp.path());

        plugins
            .for_slug("minecraft")
            .first()
            .cloned()
            .unwrap_or_else(|| panic!("no rule loaded: {:?}", plugins.errors()))
    }

    const EXEC_RULE: &str = r#"{
        "manifestVersion": 1,
        "launch": {
            "exec": "run.sh",
            "args": ["--profile", "{installName}", "--width", "{width}"],
            "optionArgs": {
                "memoryMb": ["-Xmx{memoryMb}M"],
                "windowMode": ["--{windowMode}"]
            },
            "env": { "TMC_PROFILE": "{installName}" }
        }
    }"#;

    #[test]
    fn an_unfilled_placeholder_drops_its_argument() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path();

        std::fs::write(game.join("run.sh"), "#!/bin/sh\n").expect("exe");

        let sandbox = sandbox_over(game);
        let rule = rule_from(EXEC_RULE);

        // No width configured, and no install name.
        let plan = plan(
            &rule,
            &sandbox,
            &LaunchOptions::default(),
            &LaunchContext::default(),
        )
        .expect("plan");

        // `--profile` and `--width` are literals with no placeholder of their
        // own, so they survive; their VALUES are dropped. That is the honest
        // outcome for a launch file that pairs them, and is why a file should
        // put both in `optionArgs` instead — which the next test covers.
        assert_eq!(plan.args, vec!["--profile", "--width"]);
        assert!(plan.env.is_empty());
    }

    #[test]
    fn option_args_only_appear_when_the_option_is_set() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path();

        std::fs::write(game.join("run.sh"), "#!/bin/sh\n").expect("exe");

        let sandbox = sandbox_over(game);
        let rule = rule_from(EXEC_RULE);

        let options = LaunchOptions {
            memory_mb: Some(4096),
            window_mode: Some("fullscreen".into()),
            width: Some(1920),
            ..Default::default()
        };

        let ctx = LaunchContext {
            install_name: Some("Kitchen sink".into()),
            ..Default::default()
        };

        let plan = plan(&rule, &sandbox, &options, &ctx).expect("plan");

        assert!(plan.args.contains(&"Kitchen sink".to_string()));
        assert!(plan.args.contains(&"1920".to_string()));
        assert!(plan.args.contains(&"-Xmx4096M".to_string()));
        assert!(plan.args.contains(&"--fullscreen".to_string()));
        assert_eq!(
            plan.env.get("TMC_PROFILE").map(String::as_str),
            Some("Kitchen sink")
        );
    }

    #[test]
    fn an_executable_outside_the_game_directory_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&game).expect("mkdir");

        let sandbox = sandbox_over(&game);

        for escape in ["../evil.sh", "/bin/sh", "sub/../../evil.sh"] {
            let rule = rule_from(&format!(
                r#"{{ "manifestVersion": 1, "launch": {{ "exec": "{escape}" }} }}"#
            ));

            assert!(
                plan(
                    &rule,
                    &sandbox,
                    &LaunchOptions::default(),
                    &LaunchContext::default()
                )
                .is_err(),
                "{escape} should be refused"
            );
        }
    }

    #[test]
    fn a_missing_executable_is_an_error_not_a_spawn_failure() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sandbox = sandbox_over(tmp.path());

        let rule = rule_from(r#"{ "manifestVersion": 1, "launch": { "exec": "run.sh" } }"#);

        assert!(plan(
            &rule,
            &sandbox,
            &LaunchOptions::default(),
            &LaunchContext::default()
        )
        .is_err());
    }

    #[test]
    fn shell_metacharacters_in_a_value_are_inert_text() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path();

        std::fs::write(game.join("run.sh"), "#!/bin/sh\n").expect("exe");

        let sandbox = sandbox_over(game);
        let rule = rule_from(EXEC_RULE);

        let ctx = LaunchContext {
            // The classic injection attempt, as an install NAME — which a user
            // types, so this is not hypothetical.
            install_name: Some("x; rm -rf ~ && echo $(whoami)".into()),
            ..Default::default()
        };

        let plan = plan(&rule, &sandbox, &LaunchOptions::default(), &ctx).expect("plan");

        // One argument, carrying the whole string. There is no shell to split
        // it, so it reaches the game as a single argv entry and nothing else.
        assert!(plan
            .args
            .contains(&"x; rm -rf ~ && echo $(whoami)".to_string()));
        assert_eq!(plan.args.iter().filter(|a| a.contains("rm -rf")).count(), 1);
    }

    #[test]
    fn a_newline_in_an_argument_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path();

        std::fs::write(game.join("run.sh"), "#!/bin/sh\n").expect("exe");

        let sandbox = sandbox_over(game);
        let rule = rule_from(EXEC_RULE);

        let ctx = LaunchContext {
            install_name: Some("first\nsecond".into()),
            ..Default::default()
        };

        assert!(plan(&rule, &sandbox, &LaunchOptions::default(), &ctx).is_err());
    }

    #[test]
    fn a_uri_rule_produces_no_program() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let sandbox = sandbox_over(tmp.path());

        let rule = rule_from(
            r#"{ "manifestVersion": 1, "launch": { "uri": "steam://rungameid/271590" } }"#,
        );

        let plan = plan(
            &rule,
            &sandbox,
            &LaunchOptions::default(),
            &LaunchContext::default(),
        )
        .expect("plan");

        assert_eq!(plan.uri.as_deref(), Some("steam://rungameid/271590"));
        assert!(plan.program.is_none());
    }

    #[test]
    fn describe_is_readable_and_is_not_a_shell_line() {
        let plan = LaunchPlan {
            rule: "x".into(),
            program: Some("/games/run.sh".into()),
            uri: None,
            args: vec!["--profile".into(), "Kitchen sink".into()],
            cwd: None,
            env: BTreeMap::new(),
            hide_window: false,
        };

        assert_eq!(describe(&plan), r#"/games/run.sh --profile "Kitchen sink""#);
    }
}
