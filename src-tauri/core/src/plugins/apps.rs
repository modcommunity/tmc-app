//! **App-scoped plugins** — how to install a mod for *this* game, and how to
//! launch it.
//!
//! Distinct from the plugin bundles in [`crate::plugins::registry`], and
//! deliberately so:
//!
//! | | Registry plugin | App plugin |
//! | --- | --- | --- |
//! | Where | `plugins/<id>/plugin.json` | `plugins/app/<slug>/*.json\|yaml` |
//! | Identified by | a reverse-DNS id the author picks | the GAME it handles |
//! | Approved | per bundle, by fingerprint | per game, once |
//! | Answers | "what can this plugin do?" | "where do this game's mods go?" |
//!
//! A registry plugin is a thing a user chose to install. An app plugin is a
//! *rule for a game* — "a Minecraft mod is a `.jar` that goes in `mods/`" — and
//! the app ships one for every game it supports. There are dozens of them and
//! they are all tiny, so making each one a full bundle with its own id, its own
//! approval and its own directory would be ceremony with no payoff.
//!
//! **The safety model is unchanged.** Everything here compiles down to the same
//! [`Step`] vocabulary, executed by the same [`Executor`] through the same
//! [`Sandbox`]. An app plugin cannot express anything a registry plugin cannot:
//! no shell, no absolute path, no environment read. What it adds is only the
//! *selection* — which rule applies to which game, kind and file.
//!
//! [`Executor`]: crate::plugins::steps::Executor
//! [`Sandbox`]: crate::plugins::sandbox::Sandbox
//!
//! # Layout
//!
//! ```text
//! plugins/app/
//!   minecraft/
//!     manage_mod.json
//!     manage_asset.yaml
//!     launch.json
//!     resourcepacks/manage_asset.json    ← recursive; subdirectories are fine
//!     disabled/manage_mod.json           ← IGNORED, entirely
//!   gtav/
//!     manage_mod.yaml
//! ```
//!
//! The directory name is the app's **URL slug** from the website, lower-cased
//! (`minecraft`, `gtav`) — which is unique per app and is what the API hands
//! down on every subscription. Using the slug rather than the numeric id means
//! a hand-authored file is readable, and means the same file works against a
//! development database whose ids differ.
//!
//! `disabled/` is skipped at any depth. That is the whole mechanism for turning
//! a rule off without deleting it, and it is a directory rather than a flag in
//! the file because the point is to be able to move a file without editing it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::plugins::manifest::{FsGrant, FsRoot, Manifest, Permissions, Step};

/// How deep the recursive scan goes. A rule five directories down is a mistake,
/// not a layout.
const MAX_DEPTH: u8 = 6;

/// Cap on one app plugin file. These are hand-authored rules, not data.
const MAX_FILE_BYTES: u64 = 512 * 1024;

/// Cap on how many files one game may declare, so a stray directory cannot
/// turn startup into a filesystem walk.
const MAX_FILES_PER_APP: usize = 64;

/// What a file declares.
///
/// Taken from the file's STEM, up to the first `.` — so `manage_mod.json` and
/// `manage_mod.forge.yaml` are both mod rules, which is how a game gets several
/// (see [`AppPluginFile::matches`] for how one is chosen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppPluginKind {
    ManageMod,
    ManageAsset,
    ManageCollection,
    Launch,
}

impl AppPluginKind {
    fn from_stem(stem: &str) -> Option<Self> {
        // Only the part before the first `.`, so a variant suffix is allowed.
        let head = stem.split('.').next().unwrap_or(stem);

        match head {
            "manage_mod" => Some(Self::ManageMod),
            "manage_asset" => Some(Self::ManageAsset),
            "manage_collection" => Some(Self::ManageCollection),
            "launch" => Some(Self::Launch),
            _ => None,
        }
    }

    /// The content kind this rule installs, or `None` for a launch spec.
    pub fn content_kind(self) -> Option<&'static str> {
        match self {
            Self::ManageMod => Some("mod"),
            Self::ManageAsset => Some("asset"),
            Self::ManageCollection => Some("collection"),
            Self::Launch => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManageMod => "manage_mod",
            Self::ManageAsset => "manage_asset",
            Self::ManageCollection => "manage_collection",
            Self::Launch => "launch",
        }
    }
}

/// Which subscriptions a rule applies to.
///
/// All conditions must hold. Every field is optional and an empty `Match` — the
/// default — matches everything, which is the right behaviour for a game with
/// exactly one rule.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Match {
    /// Lower-case file extensions, without the dot: `["jar"]`, `["zip", "7z"]`.
    #[serde(default)]
    pub extensions: Vec<String>,

    /// Substrings that must appear (case-insensitively) in the file name.
    /// For the games where a loader is identifiable only from the filename.
    #[serde(default)]
    pub name_contains: Vec<String>,

    /// Install loaders this rule is for (`forge`, `fabric`, …), matched against
    /// the install's `loader` field. Empty means "any".
    #[serde(default)]
    pub loaders: Vec<String>,
}

impl Match {
    /// Does this rule apply?
    ///
    /// `file_name` is the release file's name (may be empty when unknown, in
    /// which case an extension condition cannot be satisfied and the rule is
    /// skipped — better than guessing).
    pub fn applies(&self, file_name: &str, loader: Option<&str>) -> bool {
        let lower = file_name.to_ascii_lowercase();

        if !self.extensions.is_empty() {
            let ext = lower.rsplit('.').next().unwrap_or_default();

            // A dotless name has no extension; `rsplit` would otherwise hand
            // back the whole name and match `["jar"]` against a file called
            // literally `jar`.
            if !lower.contains('.') || !self.extensions.iter().any(|e| e == ext) {
                return false;
            }
        }

        if !self.name_contains.is_empty()
            && !self
                .name_contains
                .iter()
                .any(|needle| lower.contains(&needle.to_ascii_lowercase()))
        {
            return false;
        }

        if !self.loaders.is_empty() {
            let Some(loader) = loader else {
                return false;
            };

            let loader = loader.to_ascii_lowercase();

            if !self
                .loaders
                .iter()
                .any(|l| l.to_ascii_lowercase() == loader)
            {
                return false;
            }
        }

        true
    }

    /// How specific this rule is. Used to pick between two that both match: the
    /// one with more conditions wins, because it was written for a narrower
    /// case.
    fn specificity(&self) -> usize {
        self.extensions.len() + self.name_contains.len() + self.loaders.len()
    }
}

/// The permissions an app plugin declares.
///
/// The same shape as a registry plugin's, and validated identically — because
/// it is compiled into one before anything runs. Spelled out per file rather
/// than granted implicitly by being an app plugin: "this rule writes to the
/// game's `mods/` folder and downloads from our CDN" is exactly what a reader
/// of the file needs to be able to see without cross-referencing anything.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppPluginPermissions {
    #[serde(default)]
    pub fs: Vec<FsGrant>,
    #[serde(default)]
    pub net: Vec<String>,
}

/// A launch specification: how to start this game.
///
/// **This is the one place in the whole plugin system that starts a process,**
/// and it is bounded to the point of being almost inexpressive:
///
///   * `exec` is a path RELATIVE to the game directory. There is no variant for
///     an absolute path, and the same [`crate::plugins::sandbox`] rules that
///     stop an installer escaping the jail are applied to it.
///   * `uri` is the alternative for games that launch through a client
///     (`steam://rungameid/271590`). It is handed to the OS opener, never to a
///     shell, and is checked against a scheme allow-list.
///   * Arguments are a VECTOR, filled from a fixed placeholder table. There is
///     no shell, so quoting, `;`, backticks and `$()` are inert bytes.
///
/// A launch spec still requires the user's confirmation the first time it
/// resolves to a given command line — see `launch_preview` on the app side.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LaunchSpec {
    /// Executable, relative to the game directory. Mutually exclusive with
    /// `uri`; exactly one must be set.
    #[serde(default)]
    pub exec: Option<String>,

    /// Per-platform override for `exec` (`windows`, `macos`, `linux`). One game
    /// is one directory but three binaries.
    #[serde(default)]
    pub exec_platform: BTreeMap<String, String>,

    /// A URI handed to the OS (`steam://…`). Mutually exclusive with `exec`.
    #[serde(default)]
    pub uri: Option<String>,

    /// Working directory, relative to the game directory. Defaults to the game
    /// directory itself.
    #[serde(default)]
    pub cwd: Option<String>,

    /// Argument templates, in order. `{placeholder}` is filled from the launch
    /// context; an argument whose placeholders are ALL unresolved is dropped
    /// rather than passed through literally — see `LaunchPlan::build`.
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables to set for the child. Values are templates too.
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Arguments contributed by an install option, when that option is set.
    ///
    /// Keyed by the option name (`width`, `fullscreen`, `memoryMb`, …). This is
    /// what makes "resolution" mean `--width 1920 --height 1080` for one game
    /// and `-w 1920 -h 1080` for another without either being hardcoded.
    #[serde(default)]
    pub option_args: BTreeMap<String, Vec<String>>,
}

/// A manage rule: how to install and uninstall one content item for this game.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManageSpec {
    #[serde(default)]
    pub install: Vec<Step>,
    #[serde(default)]
    pub uninstall: Vec<Step>,
}

/// One parsed file under `plugins/app/<slug>/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppPluginFile {
    /// Must equal 1. Same contract as the registry manifest: a file from a
    /// newer app version is refused rather than half-understood.
    pub manifest_version: u32,

    /// Human-readable, for the log and the UI. Not an identity.
    #[serde(default)]
    pub label: Option<String>,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub r#match: Match,

    #[serde(default)]
    pub permissions: AppPluginPermissions,

    #[serde(default)]
    pub manage: Option<ManageSpec>,

    #[serde(default)]
    pub launch: Option<LaunchSpec>,

    // ---------------------------------------------------- Filled by the loader
    /// The kind, from the file name. `skip_deserializing` because it is not a
    /// key in the file — writing one would be a second, contradictable source
    /// of truth for something the path already says.
    #[serde(skip_deserializing, default = "default_kind")]
    pub kind: AppPluginKind,

    /// Path relative to `plugins/app/`, e.g. `minecraft/manage_mod.json`. The
    /// stable name this rule is identified by in the log and in the UI.
    #[serde(skip_deserializing, default)]
    pub source: String,

    /// The app slug this file was found under.
    #[serde(skip_deserializing, default)]
    pub slug: String,
}

fn default_kind() -> AppPluginKind {
    AppPluginKind::ManageMod
}

impl AppPluginFile {
    fn validate(&self) -> AppResult<()> {
        if self.manifest_version != 1 {
            return Err(AppError::invalid(format!(
                "App plugin version {} is not supported by this app version.",
                self.manifest_version
            )));
        }

        match self.kind {
            AppPluginKind::Launch => {
                let Some(launch) = &self.launch else {
                    return Err(AppError::invalid("A launch file must declare `launch`."));
                };

                let has_exec = launch.exec.is_some() || !launch.exec_platform.is_empty();

                if has_exec == launch.uri.is_some() {
                    return Err(AppError::invalid(
                        "A launch file must declare exactly one of `exec` and `uri`.",
                    ));
                }

                if let Some(uri) = &launch.uri {
                    if !is_allowed_launch_uri(uri) {
                        return Err(AppError::invalid(format!(
                            "'{uri}' is not a launch URI this app will open."
                        )));
                    }
                }

                if launch.args.len() > 64 {
                    return Err(AppError::invalid("Launch spec has too many arguments."));
                }
            }
            _ => {
                let Some(manage) = &self.manage else {
                    return Err(AppError::invalid("A manage file must declare `manage`."));
                };

                if manage.install.len() > 128 || manage.uninstall.len() > 128 {
                    return Err(AppError::invalid("App plugin has too many steps."));
                }

                /*
                 * A plan with no write grant can only fail, loudly, halfway
                 * through. Same check the registry manifest makes, for the same
                 * reason: catching it at LOAD time means the log says "this rule
                 * is broken" rather than "installing Foo failed".
                 */
                if !manage.install.is_empty() && !self.permissions.fs.iter().any(|g| g.write) {
                    return Err(AppError::invalid(
                        "App plugin has install steps but no writable filesystem permission.",
                    ));
                }
            }
        }

        Ok(())
    }

    /// Does this rule apply to a given file and install?
    pub fn matches(&self, file_name: &str, loader: Option<&str>) -> bool {
        self.r#match.applies(file_name, loader)
    }

    /// Compile into a [`Manifest`], so the existing sandbox and executor can
    /// run it unchanged.
    ///
    /// The synthetic id is derived from the source path, which is what makes
    /// `pluginData` scoped per rule — two rules for one game get separate
    /// scratch directories, and neither can read the other's staging area.
    pub fn as_manifest(&self) -> Manifest {
        Manifest {
            manifest_version: crate::plugins::manifest::MANIFEST_VERSION,
            id: self.synthetic_id(),
            name: self
                .label
                .clone()
                .unwrap_or_else(|| format!("{} ({})", self.kind.as_str(), self.slug)),
            version: "1".into(),
            author: "The Modding Community".into(),
            description: self.description.clone(),
            homepage: None,
            // App plugins are selected by SLUG, not by numeric app id — the
            // slug is what the directory name is. Leaving this empty means
            // `sandbox_for` still receives the app id from the caller, which is
            // where the game directory lookup happens.
            apps: vec![],
            permissions: Permissions {
                fs: self.permissions.fs.clone(),
                net: self.permissions.net.clone(),
                query: false,
            },
            installer: None,
            server_query: None,
            theme: None,
        }
    }

    /// A stable, path-safe id for this rule.
    ///
    /// Every character outside the plugin-id alphabet becomes `-`, so
    /// `minecraft/resourcepacks/manage_asset.yaml` becomes
    /// `app.minecraft-resourcepacks-manage-asset-yaml`. It has to satisfy
    /// `is_valid_plugin_id`, because it names a directory under `plugin-data`.
    fn synthetic_id(&self) -> String {
        let mut out = String::from("app.");

        for ch in self.source.chars() {
            if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
                out.push(ch);
            } else if ch.is_ascii_uppercase() {
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push('-');
            }
        }

        // A trailing or doubled separator is legal in the alphabet but ugly;
        // more importantly a leading/trailing `.` is REFUSED by the validator,
        // and `-` is not, which is why the replacement char is `-`.
        out.truncate(96);

        out
    }
}

/// URI schemes an app plugin may ask the OS to open in order to launch a game.
///
/// A closed list, because "open this URI" is a request to hand a string to
/// whatever program claimed a scheme — `file:` would open a local executable,
/// `http:` a browser, and a bare word could be anything. These four are game
/// clients and nothing else.
const LAUNCH_SCHEMES: [&str; 4] = [
    "steam://",
    "com.epicgames.launcher://",
    "uplay://",
    "origin://",
];

fn is_allowed_launch_uri(uri: &str) -> bool {
    let lower = uri.to_ascii_lowercase();

    if lower.len() > 512 {
        return false;
    }

    // Control characters and whitespace in a URI handed to a platform opener
    // are how one string becomes two arguments on at least one platform.
    if lower
        .chars()
        .any(|c| c.is_control() || c == '"' || c == '\'')
    {
        return false;
    }

    LAUNCH_SCHEMES.iter().any(|s| lower.starts_with(s))
}

/// Every app plugin on this machine, keyed by slug.
#[derive(Debug, Default)]
pub struct AppPlugins {
    by_slug: BTreeMap<String, Vec<AppPluginFile>>,
    /// Files that failed to parse, so Settings → Plugins can show WHY rather
    /// than silently offering fewer games than the folder contains.
    errors: Vec<(String, String)>,
}

impl AppPlugins {
    /// Scan `<root>/app/` and parse everything under it.
    ///
    /// Never fails as a whole: one broken file is recorded in `errors` and the
    /// rest load. A single malformed YAML must not cost the user every other
    /// game's install support.
    pub fn load(root: &Path) -> Self {
        let mut out = Self::default();

        let base = root.join("app");

        let Ok(entries) = std::fs::read_dir(&base) else {
            return out;
        };

        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };

            if !meta.is_dir() {
                continue;
            }

            let Some(slug) = entry.file_name().to_str().map(|s| s.to_ascii_lowercase()) else {
                continue;
            };

            if slug == "disabled" || !is_valid_slug(&slug) {
                continue;
            }

            let mut files = Vec::new();

            out.walk(&entry.path(), &slug, &slug, 0, &mut files);

            files.sort_by(|a, b| {
                // Most specific first, so `choose` can take the first match.
                b.r#match
                    .specificity()
                    .cmp(&a.r#match.specificity())
                    .then_with(|| a.source.cmp(&b.source))
            });

            if !files.is_empty() {
                out.by_slug.insert(slug, files);
            }
        }

        out
    }

    fn walk(
        &mut self,
        dir: &Path,
        slug: &str,
        rel: &str,
        depth: u8,
        files: &mut Vec<AppPluginFile>,
    ) {
        if depth > MAX_DEPTH || files.len() >= MAX_FILES_PER_APP {
            return;
        }

        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };

            // Never follow a symlink into the tree. A `disabled -> ../enabled`
            // link would otherwise re-enable everything it points at, and a
            // link out of the plugins directory would load arbitrary files.
            if meta.file_type().is_symlink() {
                continue;
            }

            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };

            if meta.is_dir() {
                // The whole disable mechanism, at any depth.
                if name.eq_ignore_ascii_case("disabled") {
                    continue;
                }

                self.walk(
                    &entry.path(),
                    slug,
                    &format!("{rel}/{name}"),
                    depth + 1,
                    files,
                );

                continue;
            }

            if meta.len() > MAX_FILE_BYTES {
                self.errors.push((
                    format!("{rel}/{name}"),
                    "File is too large to be a plugin rule.".into(),
                ));
                continue;
            }

            let source = format!("{rel}/{name}");

            match parse_file(&entry.path(), &source, slug) {
                Ok(Some(parsed)) => files.push(parsed),
                Ok(None) => {}
                Err(e) => self.errors.push((source, e.to_string())),
            }

            if files.len() >= MAX_FILES_PER_APP {
                return;
            }
        }
    }

    /// The rules for one game.
    pub fn for_slug(&self, slug: &str) -> &[AppPluginFile] {
        self.by_slug
            .get(&slug.to_ascii_lowercase())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The best rule of a kind for a given file and install, or `None`.
    ///
    /// The list is pre-sorted most-specific-first, so this is the first match —
    /// which means a rule written for `.jar` beats the catch-all, and two rules
    /// of equal specificity are broken by path so the answer is stable.
    pub fn choose(
        &self,
        slug: &str,
        kind: AppPluginKind,
        file_name: &str,
        loader: Option<&str>,
    ) -> Option<&AppPluginFile> {
        self.for_slug(slug)
            .iter()
            .find(|f| f.kind == kind && f.matches(file_name, loader))
    }

    /// The launch spec for a game, if it has one.
    pub fn launch_for(&self, slug: &str, loader: Option<&str>) -> Option<&AppPluginFile> {
        self.for_slug(slug)
            .iter()
            .find(|f| f.kind == AppPluginKind::Launch && f.matches("", loader))
    }

    /// Which games have at least one manage rule — what the UI calls "supported".
    pub fn managed_slugs(&self) -> Vec<String> {
        self.by_slug
            .iter()
            .filter(|(_, files)| files.iter().any(|f| f.kind != AppPluginKind::Launch))
            .map(|(slug, _)| slug.clone())
            .collect()
    }

    pub fn errors(&self) -> &[(String, String)] {
        &self.errors
    }

    pub fn is_empty(&self) -> bool {
        self.by_slug.is_empty()
    }
}

/// A game directory name we will look inside.
///
/// The same alphabet as an app URL on the website, which is what these mirror.
/// Strict because it becomes a path component and is read off the filesystem.
fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Parse one file. `Ok(None)` means "not a plugin rule" (a README, an icon).
fn parse_file(path: &Path, source: &str, slug: &str) -> AppResult<Option<AppPluginFile>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    let is_json = ext == "json";
    let is_yaml = ext == "yaml" || ext == "yml";

    if !is_json && !is_yaml {
        return Ok(None);
    }

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let Some(kind) = AppPluginKind::from_stem(&stem) else {
        return Ok(None);
    };

    let raw = std::fs::read_to_string(path)
        .map_err(|e| AppError::invalid(format!("Could not read the rule: {e}")))?;

    /*
     * Both formats deserialise into the SAME struct, which is the whole reason
     * both can be supported for the price of one code path. YAML is a superset
     * of JSON in principle, but not in serde's implementation of it — a JSON
     * file with a tab in its indentation parses as JSON and fails as YAML — so
     * the extension decides rather than a try-both fallback that would report
     * the wrong error.
     */
    let mut parsed: AppPluginFile = if is_json {
        serde_json::from_str(&raw).map_err(|e| AppError::invalid(format!("Invalid JSON: {e}")))?
    } else {
        serde_yaml_ng::from_str(&raw)
            .map_err(|e| AppError::invalid(format!("Invalid YAML: {e}")))?
    };

    parsed.kind = kind;
    parsed.source = source.to_string();
    parsed.slug = slug.to_string();

    parsed.validate()?;

    // The compiled manifest has to satisfy the registry validator too — that is
    // what proves an app plugin cannot express anything a registry plugin
    // cannot. Checked at LOAD time rather than at run time so a broken rule is
    // reported before a user asks it to install something.
    let manifest = parsed.as_manifest();

    if !crate::plugins::manifest::is_valid_plugin_id(&manifest.id) {
        return Err(AppError::invalid(
            "Rule path does not produce a usable plugin id.",
        ));
    }

    Ok(Some(parsed))
}

/// Where app plugins live, given the app's plugins directory.
pub fn app_plugin_dir(plugins_root: &Path) -> PathBuf {
    plugins_root.join("app")
}

/// The default grants a rule gets when it declares none.
///
/// Deliberately NOT applied automatically — a rule with no grants and install
/// steps is refused at load. This exists for the shipped examples and for the
/// docs, so "what should I write here?" has one answer.
pub fn default_grants() -> Vec<FsGrant> {
    vec![
        FsGrant {
            root: FsRoot::PluginData,
            path: String::new(),
            write: true,
        },
        FsGrant {
            root: FsRoot::GameDir,
            path: String::new(),
            write: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    const MOD_JSON: &str = r#"{
        "manifestVersion": 1,
        "label": "Minecraft jar mod",
        "match": { "extensions": ["jar"] },
        "permissions": {
            "fs": [
                { "root": "pluginData", "path": "", "write": true },
                { "root": "gameDir", "path": "mods", "write": true }
            ],
            "net": ["example.com"]
        },
        "manage": {
            "install": [
                { "action": "download", "url": "{fileUrl}", "to": { "root": "pluginData", "path": "staging/{fileName}" } },
                { "action": "copy", "from": { "root": "pluginData", "path": "staging/{fileName}" }, "to": { "root": "gameDir", "path": "mods/{fileName}" } }
            ],
            "uninstall": [
                { "action": "remove", "path": { "root": "gameDir", "path": "mods/{fileName}" } }
            ]
        }
    }"#;

    const MOD_YAML: &str = r#"
manifestVersion: 1
label: GTA V asi mod
match:
  extensions: [asi, dll]
permissions:
  fs:
    - root: pluginData
      path: ""
      write: true
    - root: gameDir
      path: ""
      write: true
manage:
  install:
    - action: download
      url: "{fileUrl}"
      to: { root: pluginData, path: "staging/{fileName}" }
    - action: copy
      from: { root: pluginData, path: "staging/{fileName}" }
      to: { root: gameDir, path: "{fileName}" }
  uninstall:
    - action: remove
      path: { root: gameDir, path: "{fileName}" }
"#;

    #[test]
    fn json_and_yaml_both_load_into_the_same_shape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "app/minecraft/manage_mod.json", MOD_JSON);
        write(root, "app/gtav/manage_mod.yaml", MOD_YAML);

        let plugins = AppPlugins::load(root);

        assert!(plugins.errors().is_empty(), "{:?}", plugins.errors());
        assert_eq!(plugins.for_slug("minecraft").len(), 1);
        assert_eq!(plugins.for_slug("gtav").len(), 1);

        let mc = plugins
            .choose("minecraft", AppPluginKind::ManageMod, "cool.jar", None)
            .expect("matched");

        assert_eq!(mc.kind, AppPluginKind::ManageMod);
        assert_eq!(mc.manage.as_ref().expect("manage").install.len(), 2);

        let gta = plugins
            .choose("gtav", AppPluginKind::ManageMod, "trainer.asi", None)
            .expect("matched");

        assert_eq!(gta.manage.as_ref().expect("manage").uninstall.len(), 1);
    }

    #[test]
    fn a_disabled_directory_is_ignored_at_any_depth() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "app/minecraft/disabled/manage_mod.json", MOD_JSON);
        write(
            root,
            "app/minecraft/packs/disabled/manage_asset.json",
            MOD_JSON,
        );

        let plugins = AppPlugins::load(root);

        assert!(plugins.for_slug("minecraft").is_empty());
    }

    #[test]
    fn subdirectories_are_scanned() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "app/minecraft/packs/manage_asset.json", MOD_JSON);

        let plugins = AppPlugins::load(root);

        let found = plugins.for_slug("minecraft");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, AppPluginKind::ManageAsset);
        assert_eq!(found[0].source, "minecraft/packs/manage_asset.json");
    }

    #[test]
    fn the_most_specific_rule_wins() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // A catch-all and a `.jar` rule. The `.jar` one must win for a jar and
        // the catch-all must still serve a zip.
        let catch_all = MOD_JSON.replace(r#""match": { "extensions": ["jar"] },"#, "");

        write(root, "app/minecraft/manage_mod.json", MOD_JSON);
        write(root, "app/minecraft/manage_mod.any.json", &catch_all);

        let plugins = AppPlugins::load(root);

        assert_eq!(plugins.for_slug("minecraft").len(), 2);

        let jar = plugins
            .choose("minecraft", AppPluginKind::ManageMod, "a.jar", None)
            .expect("matched");

        assert_eq!(jar.source, "minecraft/manage_mod.json");

        let zip = plugins
            .choose("minecraft", AppPluginKind::ManageMod, "a.zip", None)
            .expect("matched");

        assert_eq!(zip.source, "minecraft/manage_mod.any.json");
    }

    #[test]
    fn install_steps_without_a_write_grant_are_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        let no_write = MOD_JSON.replace(r#""write": true"#, r#""write": false"#);

        write(root, "app/minecraft/manage_mod.json", &no_write);

        let plugins = AppPlugins::load(root);

        assert!(plugins.for_slug("minecraft").is_empty());
        assert_eq!(plugins.errors().len(), 1);
    }

    #[test]
    fn a_broken_file_does_not_take_the_others_down() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "app/minecraft/manage_mod.json", MOD_JSON);
        write(root, "app/minecraft/manage_asset.json", "{ not json");

        let plugins = AppPlugins::load(root);

        assert_eq!(plugins.for_slug("minecraft").len(), 1);
        assert_eq!(plugins.errors().len(), 1);
    }

    #[test]
    fn launch_uris_are_restricted_to_game_clients() {
        assert!(is_allowed_launch_uri("steam://rungameid/271590"));
        assert!(is_allowed_launch_uri("uplay://launch/720/0"));

        for bad in [
            "file:///bin/sh",
            "http://example.com",
            "javascript:alert(1)",
            "steam://run\nid",
            "steam://run\"id",
            "notsteam://x",
        ] {
            assert!(!is_allowed_launch_uri(bad), "{bad} should be refused");
        }
    }

    #[test]
    fn a_launch_file_must_name_exactly_one_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(
            root,
            "app/minecraft/launch.json",
            r#"{ "manifestVersion": 1, "launch": { "exec": "run.sh", "uri": "steam://x" } }"#,
        );

        let plugins = AppPlugins::load(root);

        assert!(plugins.for_slug("minecraft").is_empty());
        assert_eq!(plugins.errors().len(), 1);
    }

    #[test]
    fn extension_matching_needs_a_real_extension() {
        let m = Match {
            extensions: vec!["jar".into()],
            ..Default::default()
        };

        assert!(m.applies("mod.jar", None));
        assert!(m.applies("MOD.JAR", None));
        // A file literally named `jar`, with no dot, must not match.
        assert!(!m.applies("jar", None));
        assert!(!m.applies("", None));
        assert!(!m.applies("mod.zip", None));
    }

    #[test]
    fn loader_conditions_need_a_loader() {
        let m = Match {
            loaders: vec!["fabric".into()],
            ..Default::default()
        };

        assert!(m.applies("a.jar", Some("Fabric")));
        assert!(!m.applies("a.jar", Some("forge")));
        assert!(!m.applies("a.jar", None));
    }

    /// The shipped examples are the documentation for this format. A field
    /// renamed without updating them would ship a reference that does not load.
    ///
    /// The teeth are in the SECOND half: parsing only proves the JSON/YAML is
    /// well-formed, while building each rule's real sandbox and resolving every
    /// step path through it is what catches a rule whose grants and step paths
    /// disagree — the exact mistake an author copying an example inherits.
    #[test]
    fn the_shipped_app_examples_all_load_and_resolve() {
        // `core/` → `src-tauri/` → repo root.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");

        let plugins = AppPlugins::load(&root);

        assert!(
            plugins.errors().is_empty(),
            "example app plugins failed to load: {:?}",
            plugins.errors()
        );

        let slugs = plugins.managed_slugs();

        assert!(
            slugs.contains(&"minecraft".to_string()) && slugs.contains(&"gtav".to_string()),
            "expected minecraft and gtav examples, found {slugs:?}"
        );

        // The `disabled/` example must not have been loaded.
        assert!(
            !plugins
                .for_slug("minecraft")
                .iter()
                .any(|f| f.source.contains("/disabled/")),
            "a rule under disabled/ was loaded"
        );

        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(&game).expect("game dir");

        let roots = crate::plugins::SandboxRoots {
            data: tmp.path().join("data"),
            cache: tmp.path().join("cache"),
        };

        let mut settings = crate::settings::AppSettings::default();
        settings
            .game_dirs
            .insert("1".into(), game.display().to_string());

        let mut checked = 0;

        for slug in &slugs {
            for rule in plugins.for_slug(slug) {
                let Some(manage) = &rule.manage else { continue };

                let manifest = rule.as_manifest();

                let sandbox = crate::plugins::sandbox_for(&manifest, &roots, &settings, Some(1))
                    .unwrap_or_else(|e| panic!("{} sandbox: {e}", rule.source));

                for step in manage.install.iter().chain(&manage.uninstall) {
                    for (path_ref, write) in step_paths(step) {
                        sandbox.resolve(path_ref, write).unwrap_or_else(|e| {
                            panic!("{} step path '{}': {e}", rule.source, path_ref.path)
                        });
                    }
                }

                checked += 1;
            }
        }

        assert!(checked >= 2, "expected several manage examples");
    }

    /// Every `PathRef` a step touches, and whether it is written to.
    fn step_paths(step: &Step) -> Vec<(&crate::plugins::manifest::PathRef, bool)> {
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
    fn synthetic_ids_are_valid_plugin_ids() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "app/minecraft/packs/manage_asset.yaml", MOD_YAML);

        let plugins = AppPlugins::load(root);
        let file = &plugins.for_slug("minecraft")[0];

        let id = file.as_manifest().id;

        assert!(
            crate::plugins::manifest::is_valid_plugin_id(&id),
            "{id} is not a valid plugin id"
        );
    }
}
