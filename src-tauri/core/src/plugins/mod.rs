pub mod apps;
pub mod config;
pub mod jail;
pub mod managers;
pub mod manifest;
pub mod query;
pub mod registry;
pub mod signature;
pub mod steps;
pub mod theme;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::{AppError, AppResult};
use crate::plugins::jail::Jail;
use crate::plugins::manifest::Manifest;
use crate::settings::AppSettings;

/// The directories a jail can be anchored to on this machine.
///
/// Supplied by the caller rather than resolved here, because resolving them
/// needs the platform's path rules — which is the Tauri crate's job. Keeping
/// the *decision* here and the *lookup* there is what lets the whole jail be
/// tested with a `tempfile::TempDir`.
pub struct JailRoots {
    /// The app's private data directory.
    pub data: PathBuf,
    /// Scratch space, cleared on launch.
    pub cache: PathBuf,
}

/// Build the jail for one run of one plugin.
///
/// Every root here comes from the app's own state — the user's configured game
/// directory, the app's data dir, the user's download dir. Nothing the plugin
/// says contributes a base path, only a subdirectory under one, which is what
/// keeps the jail's anchor outside the plugin's control.
///
/// A missing `gameDir` for the app being handled is an ERROR, not an omission:
/// falling back to a plausible default is how an installer quietly writes a
/// mod into the wrong game.
pub fn jail_for(
    manifest: &Manifest,
    roots: &JailRoots,
    settings: &AppSettings,
    app_id: Option<i64>,
) -> AppResult<Jail> {
    jail_for_scoped(manifest, roots, settings, app_id, None)
}

/// [`jail_for`], with the plugin's private directory split by `scope`.
///
/// One rule staging the same mod for two sandboxes runs the same steps with the
/// same `{fileName}` — so with one shared `pluginData` the second run's download
/// lands on the first's, and a sandbox pinned to an older version quietly gets
/// the newer file. `scope` is what keeps those apart; it is sanitised into a
/// single path component here rather than trusted, because it is built from a
/// mod key.
pub fn jail_for_scoped(
    manifest: &Manifest,
    roots: &JailRoots,
    settings: &AppSettings,
    app_id: Option<i64>,
    scope: Option<&str>,
) -> AppResult<Jail> {
    let mut available: HashMap<&'static str, PathBuf> = HashMap::new();

    let mut plugin_data = roots.data.join("plugin-data").join(&manifest.id);

    if let Some(scope) = scope {
        plugin_data = plugin_data.join(safe_scope(scope));
    }

    available.insert("pluginData", plugin_data);

    available.insert(
        "downloads",
        settings
            .download_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| roots.cache.join("downloads")),
    );

    if let Some(app_id) = app_id {
        if let Some(dir) = settings.game_dirs.get(&app_id.to_string()) {
            let path = PathBuf::from(dir);

            /*
             * The configured game directory must already exist. Creating it
             * would mean a typo in the settings silently produces a new empty
             * folder that the install then "succeeds" into, leaving the user
             * with a game that has no mods and an app that says it worked.
             */
            if !path.is_dir() {
                return Err(AppError::invalid(format!(
                    "The install folder configured for this game does not exist: {dir}"
                )));
            }

            available.insert("gameDir", path);
        }
    }

    Jail::build(manifest, &available)
}

/// One path component, from arbitrary text.
fn safe_scope(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();

    let trimmed = cleaned.trim_matches('-').to_string();

    if trimmed.is_empty() {
        "scope".into()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with(fs: Vec<manifest::FsGrant>) -> Manifest {
        Manifest {
            manifest_version: 1,
            id: "com.test.plugin".into(),
            name: "T".into(),
            version: "1".into(),
            author: "T".into(),
            description: None,
            homepage: None,
            apps: vec![],
            permissions: manifest::Permissions {
                fs,
                ..Default::default()
            },
            installer: None,
            server_query: None,
            theme: None,
            manager: None,
        }
    }

    #[test]
    fn a_missing_game_dir_is_an_error_not_a_silent_fallback() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let roots = JailRoots {
            data: tmp.path().join("data"),
            cache: tmp.path().join("cache"),
        };

        let mut settings = AppSettings::default();
        settings
            .game_dirs
            .insert("7".into(), "/definitely/not/here".into());

        let manifest = manifest_with(vec![manifest::FsGrant {
            root: manifest::FsRoot::GameDir,
            path: String::new(),
            write: true,
        }]);

        assert!(jail_for(&manifest, &roots, &settings, Some(7)).is_err());
    }

    /// Every path in every shipped example must resolve inside the jail its
    /// own manifest declares.
    ///
    /// Parsing the examples is not enough — that only proves the JSON is
    /// well-formed. This is what catches a manifest whose grants and step paths
    /// disagree, which is exactly the mistake an author copying the example
    /// would inherit.
    #[test]
    fn every_example_installers_paths_resolve_inside_its_own_grants() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");

        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&game).expect("game dir");

        let roots = JailRoots {
            data: tmp.path().join("data"),
            cache: tmp.path().join("cache"),
        };

        let mut checked = 0;

        for entry in std::fs::read_dir(&root).expect("examples").flatten() {
            let path = entry.path().join(manifest::MANIFEST_FILE);

            if !path.exists() {
                continue;
            }

            let raw = std::fs::read_to_string(&path).expect("readable");
            let parsed = Manifest::parse(&raw).expect("valid manifest");

            let Some(installer) = &parsed.installer else {
                continue;
            };

            let app_id = parsed.apps.first().copied();

            let mut settings = AppSettings::default();

            if let Some(id) = app_id {
                settings
                    .game_dirs
                    .insert(id.to_string(), game.display().to_string());
            }

            let jail = jail_for(&parsed, &roots, &settings, app_id)
                .unwrap_or_else(|e| panic!("{} jail: {e}", parsed.id));

            for step in installer.install.iter().chain(&installer.uninstall) {
                for (path_ref, write) in step_paths(step) {
                    jail.resolve(path_ref, write).unwrap_or_else(|e| {
                        panic!("{} step path '{}': {e}", parsed.id, path_ref.path)
                    });
                }
            }

            checked += 1;
        }

        assert!(checked >= 1, "expected at least one installer example");
    }

    /// Every `PathRef` a step touches, and whether it is written to.
    fn step_paths(step: &manifest::Step) -> Vec<(&manifest::PathRef, bool)> {
        use manifest::Step;

        match step {
            Step::Download { to, .. } => vec![(to, true)],
            Step::Extract { from, to, .. } => vec![(from, false), (to, true)],
            Step::Copy { from, to } => vec![(from, false), (to, true)],
            Step::Move { from, to } => vec![(from, true), (to, true)],
            Step::Mkdir { path }
            | Step::Remove { path }
            | Step::WriteText { path, .. }
            | Step::PatchJson { path, .. } => vec![(path, true)],
        }
    }

    #[test]
    fn plugin_data_is_scoped_to_the_plugin_id() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let roots = JailRoots {
            data: tmp.path().join("data"),
            cache: tmp.path().join("cache"),
        };

        let manifest = manifest_with(vec![manifest::FsGrant {
            root: manifest::FsRoot::PluginData,
            path: String::new(),
            write: true,
        }]);

        let jail = jail_for(&manifest, &roots, &AppSettings::default(), None).expect("builds");

        let root = jail
            .root_path(manifest::FsRoot::PluginData)
            .expect("granted");

        assert!(root.ends_with("com.test.plugin"));
    }
}
