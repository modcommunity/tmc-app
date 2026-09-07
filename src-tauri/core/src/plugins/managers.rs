//! **The fourth plugin type: a mod manager this app can read.**
//!
//! Somebody arriving here has years of mods already installed, and almost
//! certainly through Vortex, Mod Organizer, CurseForge, r2modman or Prism. The
//! honest way to earn their list is not to ask them to rebuild it. It is to
//! read the one their manager already keeps.
//!
//! WHY THIS IS A PLUGIN AND NOT A MODULE PER MANAGER
//! ------------------------------------------------
//! There are dozens of managers and the popular ones change their storage
//! layout between major versions. A `vortex.rs` in this crate would mean a
//! release of the whole app every time one of them moved a folder — and a user
//! whose manager moved is exactly the user who cannot wait for one. A
//! descriptor is a file: shipped ones are compiled in, and a user or a
//! community can drop a corrected one in `<app data>/plugins/manager/` and have
//! it work immediately.
//!
//! It is also the same bargain every other plugin here makes, restated: **a
//! plugin is data, never code.** A manager descriptor names directories and
//! says how they nest. It cannot run a program, read an environment variable,
//! name an absolute path or reach the network, because there is nothing in the
//! format that can express any of those.
//!
//! WHAT IT CAN SAY
//! ---------------
//! ```json
//! {
//!   "manifestVersion": 1,
//!   "manager": {
//!     "id": "r2modman",
//!     "label": "r2modman / Thunderstore Mod Manager",
//!     "roots": [
//!       { "platform": "windows", "base": "appData", "path": "r2modmanPlus-local" },
//!       { "platform": "linux",   "base": "config",  "path": "r2modmanPlus-local" }
//!     ],
//!     "layout": { "modsPath": "{game}/profiles/*/BepInEx/plugins", "entry": "directory" },
//!     "games": [
//!       { "dir": "Valheim", "slug": "valheim", "relPath": "BepInEx/plugins" }
//!     ],
//!     "metadata": { "file": "manifest.json", "name": "/name", "version": "/version_number" }
//!   }
//! }
//! ```
//!
//! Every path is relative to a base **the app resolves** — the same rule the
//! plugin jail follows, and for the same reason: a descriptor that could name
//! `C:\` would be a directory-listing primitive with a JSON file for a syntax.
//!
//! THE ONE WILDCARD, AND WHY THERE IS EXACTLY ONE
//! ---------------------------------------------
//! `modsPath` may contain a single `*` segment, which expands to every
//! subdirectory at that position and whose name becomes the candidate's
//! [`ManagerMod::group`] — an r2modman profile, an MO2 instance, a Prism
//! instance. One, not any number: two would make the scan's cost the product of
//! two directory listings on somebody's whole `AppData`, and no manager
//! studied for this needs a second.
//!
//! WHAT ABOUT THE PUBLIC APIS?
//! ---------------------------
//! Asked for, and the answer is worth stating plainly rather than shipping
//! something that half-works:
//!
//!   * **CurseForge** has a REST API, and it requires a key issued per
//!     application whose terms cover distribution of *their* metadata. It also
//!     does not answer "what has this user installed" — that is in the local
//!     `minecraftinstance.json`, which this reads.
//!   * **Nexus / Vortex** likewise: the Nexus API is keyed per user, and Vortex
//!     itself has no remote API at all. What it has is a `state` database and a
//!     mods folder per game, which this reads.
//!   * **Thunderstore's** API is open, but it serves the *registry* — what
//!     exists — not what a particular machine installed. r2modman's profiles
//!     are on disk, which is what this reads.
//!
//! So every shipped descriptor reads local storage. That is not a compromise
//! forced by the plugin model: it is where the answer actually is. When a
//! manager does publish an installed-mods API, the descriptor format gains a
//! field and the built-in list gains a row — and the import path below, which
//! takes "a name and a folder", does not change at all.
//!
//! WHAT A SCAN NEVER DOES
//! ----------------------
//! Modify anything. A candidate is a report; importing one COPIES the files
//! into this app's own store (see [`crate::local::store`]) and leaves the other
//! manager's copy exactly where it was. Somebody trying this app out must be
//! able to go back to Vortex the next morning and find nothing moved.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::detect::DetectRoots;
use crate::error::{AppError, AppResult};

include!(concat!(env!("OUT_DIR"), "/builtin_managers.rs"));

/// Cap on the directories one scan will list.
///
/// The scan reads two or three levels of somebody's `AppData`, and every bound
/// here exists for a real machine rather than a hypothetical one: a Vortex
/// install with fifteen games and two thousand mods is normal, and a `*`
/// segment pointed at a directory that turned out to hold ten thousand entries
/// is what an unbounded version does on it.
pub const MAX_SCAN_DIRS: usize = 4_000;

/// Cap on the candidates one scan reports.
pub const MAX_CANDIDATES: usize = 2_000;

/// Cap on how deep a `modsPath` may reach.
pub const MAX_PATH_SEGMENTS: usize = 12;

/// Cap on a per-mod metadata file.
const MAX_METADATA_BYTES: u64 = 1024 * 1024;

/// A directory base the APP resolves. A descriptor picks one; it never supplies
/// a path of its own to start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManagerBase {
    Home,
    /// `%APPDATA%` on Windows, `~/.config` elsewhere. Tauri's `config_dir`.
    AppData,
    /// Same directory, under the name the non-Windows world uses for it. Both
    /// spellings exist because a descriptor reads better with the one its
    /// manager's own documentation uses.
    Config,
    /// `%LOCALAPPDATA%`, `~/.local/share`.
    LocalData,
    /// `%ProgramData%`.
    ProgramData,
    /// `~/Documents`. Where several Windows-first managers still put things.
    Documents,
}

impl ManagerBase {
    fn resolve(self, roots: &DetectRoots) -> Option<PathBuf> {
        match self {
            Self::Home => roots.home.clone(),
            Self::AppData | Self::Config => roots.config.clone(),
            Self::LocalData => roots.local_data.clone(),
            Self::ProgramData => roots.program_data.clone(),
            Self::Documents => roots.home.as_ref().map(|h| h.join("Documents")),
        }
    }
}

/// One place a manager might keep its data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerRoot {
    /// `windows`, `macos` or `linux`. Absent means every platform, which is
    /// right for a manager that uses the same relative layout everywhere.
    #[serde(default)]
    pub platform: Option<String>,
    pub base: ManagerBase,
    /// Relative to `base`. May be empty.
    #[serde(default)]
    pub path: String,
}

impl ManagerRoot {
    fn applies_here(&self) -> bool {
        let Some(declared) = &self.platform else {
            return true;
        };

        let current = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };

        declared.eq_ignore_ascii_case(current)
    }
}

/// How mods sit under one root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerLayout {
    /// Relative to the root. `{game}` is replaced with the game's directory
    /// name; a single `*` segment expands to every subdirectory there.
    pub mods_path: String,

    /// Whether each entry at the end of `modsPath` is a folder of files or a
    /// single archive.
    #[serde(default)]
    pub entry: EntryKind,

    /// Directory names to skip — a manager's own bookkeeping folders.
    #[serde(default)]
    pub ignore: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    #[default]
    Directory,
    Archive,
}

/// One game the manager and this app both know about.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerGame {
    /// The manager's own directory name for it, as it appears under the root.
    pub dir: String,
    /// The game's URL slug on TMC — what selects `plugins/app/<slug>/…`.
    pub slug: String,
    /// Where a mod from this manager belongs under the GAME folder, when the
    /// manager's layout implies one. Absent falls back to the game's own
    /// `sandbox.json` mod target, which is the same answer an ordinary import
    /// gets.
    #[serde(default)]
    pub rel_path: Option<String>,
}

/// An optional per-mod description file the manager writes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataSpec {
    /// A JSON file inside each mod's folder. One path component — a metadata
    /// file that could be `../../secrets.json` is a file-read primitive.
    pub file: String,

    /// JSON pointers into that file. Absent means "this manager does not record
    /// it", which is different from "it is empty".
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
}

/// A manager this app can read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagerSpec {
    /// Stable, lower-case, and the identity everything else uses.
    pub id: String,
    pub label: String,

    #[serde(default)]
    pub homepage: Option<String>,

    /// Shown verbatim when this manager is selected. Where a descriptor says
    /// the things a user has to know — "point this at your instance folder",
    /// "profiles are per-game here".
    #[serde(default)]
    pub notes: Vec<String>,

    pub roots: Vec<ManagerRoot>,
    pub layout: ManagerLayout,

    #[serde(default)]
    pub games: Vec<ManagerGame>,

    #[serde(default)]
    pub metadata: Option<MetadataSpec>,
}

/// The wrapper a descriptor file uses, so a shipped file and a registry
/// plugin's `manager` block are the same bytes.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManagerFile {
    manifest_version: u32,
    manager: ManagerSpec,
}

impl ManagerSpec {
    /// Parse and validate one descriptor.
    ///
    /// Validation is not a formality here: every field below becomes part of a
    /// path this app then lists. A descriptor that passes may name directories;
    /// one that does not, names nothing.
    pub fn parse(raw: &str) -> AppResult<Self> {
        let file: ManagerFile = if raw.trim_start().starts_with('{') {
            serde_json::from_str(raw)
                .map_err(|e| AppError::invalid(format!("Not a readable manager plugin: {e}")))?
        } else {
            serde_yaml_ng::from_str(raw)
                .map_err(|e| AppError::invalid(format!("Not a readable manager plugin: {e}")))?
        };

        if file.manifest_version != crate::plugins::manifest::MANIFEST_VERSION {
            return Err(AppError::invalid(
                "This manager plugin was written for a different version of the app.",
            ));
        }

        file.manager.validated()
    }

    fn validated(self) -> AppResult<Self> {
        if !is_valid_id(&self.id) {
            return Err(AppError::invalid(
                "A manager id must be lower-case letters, digits, '-' or '_'.",
            ));
        }

        if self.label.trim().is_empty() || self.label.len() > 80 {
            return Err(AppError::invalid("A manager needs a short label."));
        }

        if self.roots.is_empty() {
            return Err(AppError::invalid(
                "A manager plugin has to say where its data lives.",
            ));
        }

        for root in &self.roots {
            check_relative(&root.path)?;
        }

        let segments = split_path(&self.layout.mods_path);

        if segments.len() > MAX_PATH_SEGMENTS {
            return Err(AppError::invalid("That mods path is too deep."));
        }

        if segments.iter().filter(|s| *s == "*").count() > 1 {
            return Err(AppError::invalid(
                "A mods path may contain at most one '*' segment.",
            ));
        }

        for segment in &segments {
            if segment == "*" || segment.contains("{game}") {
                continue;
            }

            check_relative(segment)?;
        }

        for game in &self.games {
            check_relative(&game.dir)?;

            if let Some(rel) = &game.rel_path {
                check_relative(rel)?;
            }

            if game.slug.trim().is_empty() {
                return Err(AppError::invalid("Every game needs a TMC slug."));
            }
        }

        if let Some(meta) = &self.metadata {
            if meta.file.contains('/') || meta.file.contains('\\') {
                return Err(AppError::invalid(
                    "A metadata file must be one name, not a path.",
                ));
            }

            check_relative(&meta.file)?;

            for pointer in [&meta.name, &meta.version, &meta.author, &meta.website]
                .into_iter()
                .flatten()
            {
                if !pointer.starts_with('/') || pointer.len() > 128 {
                    return Err(AppError::invalid(
                        "A metadata pointer must start with '/' and be short.",
                    ));
                }
            }
        }

        Ok(self)
    }

    /// Which TMC game a manager directory name maps to.
    pub fn game_for(&self, dir: &str) -> Option<&ManagerGame> {
        self.games.iter().find(|g| g.dir.eq_ignore_ascii_case(dir))
    }
}

/// One installed mod, as another manager holds it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerMod {
    /// The manager it came from.
    pub manager: String,
    /// The game, in this app's vocabulary.
    pub slug: String,
    /// The manager's own directory name for the game, for the label.
    pub game_dir: String,
    /// The profile or instance, when the layout has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,

    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,

    /// Where it belongs under the GAME folder, from the descriptor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel_path: Option<String>,

    pub is_dir: bool,
    pub files: usize,
    pub bytes: u64,

    /// The absolute path, `skip`ped on the wire.
    ///
    /// The webview names a candidate by token, never by path — the same rule a
    /// dropped file follows. See `commands::import`.
    #[serde(skip)]
    pub path: PathBuf,
}

/// Read one manager's storage.
///
/// Never fails as a whole: a root that does not exist is a manager that is not
/// installed, which is the normal case for four of the five shipped
/// descriptors on any given machine.
pub fn scan(spec: &ManagerSpec, roots: &DetectRoots) -> Vec<ManagerMod> {
    let mut out = Vec::new();
    let mut listed = 0usize;

    for root in &spec.roots {
        if !root.applies_here() {
            continue;
        }

        let Some(base) = root.base.resolve(roots) else {
            continue;
        };

        let start = match join_checked(&base, &root.path) {
            Ok(path) => path,
            Err(_) => continue,
        };

        if !start.is_dir() {
            continue;
        }

        for game in &spec.games {
            if out.len() >= MAX_CANDIDATES || listed >= MAX_SCAN_DIRS {
                return out;
            }

            collect_game(spec, game, &start, &mut out, &mut listed);
        }
    }

    out.sort_by(|a, b| {
        a.slug
            .cmp(&b.slug)
            .then_with(|| a.group.cmp(&b.group))
            .then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
    });

    out
}

fn collect_game(
    spec: &ManagerSpec,
    game: &ManagerGame,
    root: &Path,
    out: &mut Vec<ManagerMod>,
    listed: &mut usize,
) {
    let segments = split_path(&spec.layout.mods_path);

    // Everything up to the wildcard, the wildcard, and everything after it.
    let star = segments.iter().position(|s| s == "*");

    let (before, after) = match star {
        Some(index) => (&segments[..index], &segments[index + 1..]),
        None => (&segments[..], &segments[segments.len()..]),
    };

    let mut head = root.to_path_buf();

    for segment in before {
        let filled = segment.replace("{game}", &game.dir);

        match join_checked(&head, &filled) {
            Ok(next) => head = next,
            Err(_) => return,
        }
    }

    if star.is_none() {
        collect_entries(spec, game, &head, None, out, listed);

        return;
    }

    let Ok(entries) = std::fs::read_dir(&head) else {
        return;
    };

    for entry in entries.flatten() {
        if *listed >= MAX_SCAN_DIRS || out.len() >= MAX_CANDIDATES {
            return;
        }

        *listed += 1;

        if !entry.metadata().is_ok_and(|m| m.is_dir()) {
            continue;
        }

        let Some(group) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };

        if spec
            .layout
            .ignore
            .iter()
            .any(|i| i.eq_ignore_ascii_case(&group))
        {
            continue;
        }

        let mut tail = entry.path();
        let mut ok = true;

        for segment in after {
            let filled = segment.replace("{game}", &game.dir);

            match join_checked(&tail, &filled) {
                Ok(next) => tail = next,
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }

        if ok {
            collect_entries(spec, game, &tail, Some(group), out, listed);
        }
    }
}

fn collect_entries(
    spec: &ManagerSpec,
    game: &ManagerGame,
    dir: &Path,
    group: Option<String>,
    out: &mut Vec<ManagerMod>,
    listed: &mut usize,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if *listed >= MAX_SCAN_DIRS || out.len() >= MAX_CANDIDATES {
            return;
        }

        *listed += 1;

        // A manager's own deployment is symlinks into its staging folder; the
        // staging folder is what this reads, and following a link out of it
        // would import the game's copy instead of the manager's.
        if entry.file_type().is_ok_and(|t| t.is_symlink()) {
            continue;
        }

        let Ok(meta) = entry.metadata() else { continue };

        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };

        if name.starts_with('.')
            || spec
                .layout
                .ignore
                .iter()
                .any(|i| i.eq_ignore_ascii_case(&name))
        {
            continue;
        }

        let wants_dir = spec.layout.entry == EntryKind::Directory;

        if meta.is_dir() != wants_dir {
            continue;
        }

        let (files, bytes) = if meta.is_dir() {
            measure(&entry.path())
        } else {
            (1, meta.len())
        };

        if files == 0 {
            continue;
        }

        let described = spec
            .metadata
            .as_ref()
            .filter(|_| meta.is_dir())
            .and_then(|m| read_metadata(m, &entry.path()))
            .unwrap_or_default();

        out.push(ManagerMod {
            manager: spec.id.clone(),
            slug: game.slug.clone(),
            game_dir: game.dir.clone(),
            group: group.clone(),
            name: described
                .name
                .unwrap_or_else(|| clean_name(&name, meta.is_dir())),
            version: described.version,
            author: described.author,
            website: described.website,
            rel_path: game.rel_path.clone(),
            is_dir: meta.is_dir(),
            files,
            bytes,
            path: entry.path(),
        });
    }
}

/// What a manager's own per-mod file said.
#[derive(Debug, Default)]
struct Described {
    name: Option<String>,
    version: Option<String>,
    author: Option<String>,
    website: Option<String>,
}

fn read_metadata(spec: &MetadataSpec, dir: &Path) -> Option<Described> {
    let path = crate::plugins::jail::join_relative(dir, &spec.file).ok()?;

    let meta = std::fs::metadata(&path).ok()?;

    if !meta.is_file() || meta.len() > MAX_METADATA_BYTES {
        return None;
    }

    let raw = std::fs::read(&path).ok()?;
    let doc: serde_json::Value = serde_json::from_slice(&raw).ok()?;

    fn at(doc: &serde_json::Value, pointer: &Option<String>, max: usize) -> Option<String> {
        let pointer = pointer.as_deref()?;

        let value = doc.pointer(pointer)?;

        let text = match value {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            // An author field that is an array of names is common enough to be
            // worth handling; anything else is not text and is skipped.
            serde_json::Value::Array(items) => items
                .iter()
                .filter_map(|i| i.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            _ => return None,
        };

        let cleaned: String = text.chars().filter(|c| !c.is_control()).take(max).collect();

        if cleaned.trim().is_empty() {
            None
        } else {
            Some(cleaned.trim().to_string())
        }
    }

    Some(Described {
        name: at(&doc, &spec.name, 200),
        version: at(&doc, &spec.version, 64),
        author: at(&doc, &spec.author, 120),
        website: at(&doc, &spec.website, 512),
    })
}

/// Every manager descriptor this build knows about: the ones compiled in, with
/// the user's own `<plugins>/manager/` overlaid by id.
///
/// Compiled in rather than shipped beside the binary, for exactly the reason
/// the app rules are — see `core/build.rs`. A portable Windows exe and a
/// relocated AppImage carry no resources, and a manager list that is silently
/// empty looks identical to "you have no mod managers installed".
pub fn resolve(plugins_root: &Path) -> (Vec<ManagerSpec>, Vec<(String, String)>) {
    let mut by_id: BTreeMap<String, ManagerSpec> = BTreeMap::new();
    let mut errors = Vec::new();

    for (name, raw) in BUILTIN_MANAGERS {
        match ManagerSpec::parse(raw) {
            Ok(spec) => {
                by_id.insert(spec.id.clone(), spec);
            }
            Err(e) => errors.push(((*name).to_string(), e.to_string())),
        }
    }

    let dir = plugins_root.join("manager");

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }

            if !entry.metadata().is_ok_and(|m| m.is_file()) {
                continue;
            }

            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();

            if !(lower.ends_with(".json") || lower.ends_with(".yaml") || lower.ends_with(".yml")) {
                continue;
            }

            let Ok(raw) = std::fs::read_to_string(entry.path()) else {
                continue;
            };

            match ManagerSpec::parse(&raw) {
                // Whole-id replacement, not a merge — the same rule an app rule
                // follows, and for the same reason: two authors' descriptors
                // for one manager interleaved would scan a layout neither of
                // them wrote.
                Ok(spec) => {
                    by_id.insert(spec.id.clone(), spec);
                }
                Err(e) => errors.push((name, e.to_string())),
            }
        }
    }

    (by_id.into_values().collect(), errors)
}

// ------------------------------------------------------------------ Helpers

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Every path a descriptor supplies goes through the jail's own rules.
///
/// Not because a manager scan writes anything — it never does — but because
/// these paths are `read_dir`'d under the user's home directory, and `..` in a
/// descriptor is how a directory listing of somewhere else gets requested.
fn check_relative(raw: &str) -> AppResult<()> {
    if raw.is_empty() {
        return Ok(());
    }

    crate::plugins::jail::join_relative(Path::new("/tmc-check"), raw)?;

    Ok(())
}

fn join_checked(base: &Path, relative: &str) -> AppResult<PathBuf> {
    if relative.is_empty() {
        return Ok(base.to_path_buf());
    }

    crate::plugins::jail::join_relative(base, relative)
}

fn split_path(raw: &str) -> Vec<String> {
    raw.replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .map(str::to_owned)
        .collect()
}

fn clean_name(raw: &str, is_dir: bool) -> String {
    if is_dir {
        return raw.to_string();
    }

    raw.rsplit_once('.')
        .map(|(head, _)| head)
        .filter(|h| !h.is_empty())
        .unwrap_or(raw)
        .to_string()
}

fn measure(dir: &Path) -> (usize, u64) {
    fn inner(dir: &Path, depth: usize, files: &mut usize, bytes: &mut u64) {
        if depth > 16 || *files > 50_000 {
            return;
        }

        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }

            let Ok(meta) = entry.metadata() else { continue };

            if meta.is_dir() {
                inner(&entry.path(), depth + 1, files, bytes);
            } else if meta.is_file() {
                *files += 1;
                *bytes += meta.len();
            }
        }
    }

    let mut files = 0usize;
    let mut bytes = 0u64;

    inner(dir, 0, &mut files, &mut bytes);

    (files, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(home: &Path) -> DetectRoots {
        DetectRoots {
            home: Some(home.to_path_buf()),
            config: Some(home.join(".config")),
            local_data: Some(home.join(".local/share")),
            program_data: None,
            program_files: Vec::new(),
            drives: Vec::new(),
        }
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }

        std::fs::write(path, body).expect("write");
    }

    const VORTEX: &str = r#"{
        "manifestVersion": 1,
        "manager": {
            "id": "vortex",
            "label": "Vortex",
            "roots": [{ "base": "appData", "path": "Vortex" }],
            "layout": { "modsPath": "{game}/mods", "entry": "directory" },
            "games": [{ "dir": "valheim", "slug": "valheim", "relPath": "BepInEx/plugins" }]
        }
    }"#;

    const R2: &str = r#"{
        "manifestVersion": 1,
        "manager": {
            "id": "r2modman",
            "label": "r2modman",
            "roots": [{ "base": "config", "path": "r2modmanPlus-local" }],
            "layout": { "modsPath": "{game}/profiles/*/BepInEx/plugins", "entry": "directory" },
            "games": [{ "dir": "Valheim", "slug": "valheim", "relPath": "BepInEx/plugins" }],
            "metadata": { "file": "manifest.json", "name": "/name", "version": "/version_number" }
        }
    }"#;

    #[test]
    fn a_flat_layout_finds_every_mod_folder() {
        let home = tempfile::tempdir().expect("tempdir");
        let spec = ManagerSpec::parse(VORTEX).expect("parse");

        let mods = home.path().join(".config/Vortex/valheim/mods");

        write(&mods.join("BetterUI/BetterUI.dll"), "x");
        write(&mods.join("MoreSlots/MoreSlots.dll"), "y");

        let found = scan(&spec, &roots(home.path()));

        let names: Vec<&str> = found.iter().map(|m| m.name.as_str()).collect();

        assert_eq!(names, vec!["BetterUI", "MoreSlots"]);
        assert_eq!(found[0].slug, "valheim");
        assert_eq!(found[0].rel_path.as_deref(), Some("BepInEx/plugins"));
        assert_eq!(found[0].group, None);
    }

    /// The wildcard is what makes profile-per-game managers readable at all.
    #[test]
    fn a_wildcard_segment_becomes_the_profile_name() {
        let home = tempfile::tempdir().expect("tempdir");
        let spec = ManagerSpec::parse(R2).expect("parse");

        let base = home
            .path()
            .join(".config/r2modmanPlus-local/Valheim/profiles");

        write(&base.join("Default/BepInEx/plugins/ModA/a.dll"), "x");
        write(
            &base.join("Default/BepInEx/plugins/ModA/manifest.json"),
            r#"{"name":"Proper Name","version_number":"1.2.3"}"#,
        );
        write(&base.join("Hardcore/BepInEx/plugins/ModB/b.dll"), "y");

        let found = scan(&spec, &roots(home.path()));

        assert_eq!(found.len(), 2);

        let a = found
            .iter()
            .find(|m| m.group.as_deref() == Some("Default"))
            .expect("a");

        // The manager's own metadata wins over the folder name.
        assert_eq!(a.name, "Proper Name");
        assert_eq!(a.version.as_deref(), Some("1.2.3"));

        let b = found
            .iter()
            .find(|m| m.group.as_deref() == Some("Hardcore"))
            .expect("b");

        assert_eq!(b.name, "ModB");
        assert_eq!(b.version, None);
    }

    #[test]
    fn a_manager_that_is_not_installed_scans_to_nothing() {
        let home = tempfile::tempdir().expect("tempdir");
        let spec = ManagerSpec::parse(VORTEX).expect("parse");

        assert!(scan(&spec, &roots(home.path())).is_empty());
    }

    /// A descriptor is data from outside the app. These are the shapes that
    /// must never load, because each of them turns a scan into a directory
    /// listing of somewhere the user did not point it.
    #[test]
    fn a_descriptor_cannot_escape_its_base() {
        let bad = [
            r#"{"manifestVersion":1,"manager":{"id":"x","label":"X","roots":[{"base":"home","path":"../../etc"}],"layout":{"modsPath":"mods"}}}"#,
            r#"{"manifestVersion":1,"manager":{"id":"x","label":"X","roots":[{"base":"home","path":"ok"}],"layout":{"modsPath":"../up/mods"}}}"#,
            r#"{"manifestVersion":1,"manager":{"id":"x","label":"X","roots":[{"base":"home","path":"ok"}],"layout":{"modsPath":"mods"},"games":[{"dir":"../..","slug":"y"}]}}"#,
            r#"{"manifestVersion":1,"manager":{"id":"x","label":"X","roots":[{"base":"home","path":"ok"}],"layout":{"modsPath":"mods"},"metadata":{"file":"../secrets.json"}}}"#,
        ];

        for raw in bad {
            assert!(
                ManagerSpec::parse(raw).is_err(),
                "should have been refused: {raw}"
            );
        }
    }

    #[test]
    fn only_one_wildcard_is_allowed() {
        let raw = r#"{"manifestVersion":1,"manager":{"id":"x","label":"X","roots":[{"base":"home","path":"ok"}],"layout":{"modsPath":"*/*/mods"}}}"#;

        assert!(ManagerSpec::parse(raw).is_err());
    }

    #[test]
    fn a_future_manifest_version_is_refused() {
        let raw = r#"{"manifestVersion":2,"manager":{"id":"x","label":"X","roots":[{"base":"home","path":"ok"}],"layout":{"modsPath":"mods"}}}"#;

        assert!(ManagerSpec::parse(raw).is_err());
    }

    /// A manager's deployment into the game folder is symlinks. Following one
    /// would import the game's copy and leave the manager's — the same file,
    /// found in the wrong place, with the wrong name.
    #[cfg(unix)]
    #[test]
    fn symlinked_entries_are_skipped() {
        let home = tempfile::tempdir().expect("tempdir");
        let spec = ManagerSpec::parse(VORTEX).expect("parse");

        let mods = home.path().join(".config/Vortex/valheim/mods");

        write(&mods.join("Real/a.dll"), "x");
        std::fs::create_dir_all(home.path().join("elsewhere")).expect("mkdir");
        std::os::unix::fs::symlink(home.path().join("elsewhere"), mods.join("Linked"))
            .expect("symlink");

        let found = scan(&spec, &roots(home.path()));

        let names: Vec<&str> = found.iter().map(|m| m.name.as_str()).collect();

        assert_eq!(names, vec!["Real"]);
    }

    /// Every shipped descriptor has to be valid, and the compiled-in set has to
    /// be non-empty — an empty one looks exactly like "you have no managers
    /// installed", which is the failure mode `core/build.rs` exists to prevent.
    #[test]
    fn every_shipped_manager_descriptor_parses() {
        assert!(
            !BUILTIN_MANAGERS.is_empty(),
            "no manager descriptors were compiled in"
        );

        for (name, raw) in BUILTIN_MANAGERS {
            ManagerSpec::parse(raw)
                .unwrap_or_else(|e| panic!("shipped manager {name} did not parse: {e}"));
        }
    }

    #[test]
    fn shipped_manager_ids_are_unique() {
        let (specs, errors) = resolve(Path::new("/nonexistent"));

        assert!(
            errors.is_empty(),
            "shipped descriptors had errors: {errors:?}"
        );

        let mut ids: Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        let before = ids.len();

        ids.sort_unstable();
        ids.dedup();

        assert_eq!(before, ids.len(), "two shipped descriptors share an id");
    }
}
