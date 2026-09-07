//! The **local content store**: mods that are on this machine without an
//! account behind them.
//!
//! Everything else the app installs starts as a subscription — the account says
//! "keep this", the sync loop mirrors the row, and the installer materialises
//! it. That covers what the site holds and nothing else. It does not cover the
//! jar somebody was handed on Discord, the folder they built themselves, the
//! pack they downloaded before they had an account, or the two hundred mods
//! already sitting in Vortex.
//!
//! A **local mod** is one directory of files, laid out exactly as they should
//! appear under a game folder, plus a row of editable text about it. That is
//! the whole model, and it is deliberately the same shape a staged
//! subscription already has — [`crate::deploy::DeployMod`] takes a name, a
//! priority and a root, and does not care which of the two produced it. A local
//! mod therefore merges, conflicts, deploys, backs up and purges through the
//! identical code path, and there is no second deployment story to keep
//! correct.
//!
//! WHERE THE FILES LIVE
//! --------------------
//! ```text
//!   <data>/local-mods/<id>/          the deploy root — game-folder-relative
//!   <data>/local-mods/.incoming/…    where an import is assembled
//! ```
//!
//! Under the app's own data directory rather than inside a sandbox's staging
//! folder, and that is load-bearing: one imported mod belongs to the DEVICE and
//! is routinely in several sandboxes at once. Copying it per sandbox would put
//! the same gigabyte on disk four times and make "update the copy I imported"
//! four operations. It is also why the store is never written during a deploy —
//! the same rule staging follows, and what lets two sandboxes share one import.
//!
//! WHAT AN IMPORT IS ALLOWED TO DECIDE
//! -----------------------------------
//! Where the payload lands under the game folder — its `rel_path` — is a guess,
//! and the module is honest about that rather than clever. Three inputs, in
//! descending order of authority:
//!
//!   1. the archive's own [`ModMetadata::install_path`], when it carries one;
//!   2. the payload already looking game-root-relative, i.e. its top level
//!      contains a directory the game's `sandbox.json` declares as a mod target;
//!   3. the game's first declared mod target.
//!
//! And then the user can change it, because a guess about somebody else's
//! archive is a guess. [`relayout`] is what makes that cheap: it is a directory
//! rename inside the store, not a re-import.
//!
//! WHAT AN IMPORT NEVER DOES
//! -------------------------
//! Reach the network, run anything, or read a path the webview named. The
//! source path always comes from something Rust itself observed — an OS drop
//! event, a scan this crate performed — and the caller names it by token. See
//! `commands::import` for that half.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::plugins::jail::join_relative;

use super::metadata::{ModMetadata, METADATA_FILE, METADATA_FILE_ALT};

/// Cap on the files one import may produce.
///
/// Generous — a Skyrim texture pack is tens of thousands of loose files — and
/// still bounded, because the copy below is the one place a pathological folder
/// turns into an unresponsive app.
pub const MAX_IMPORT_FILES: usize = 100_000;

/// Cap on the bytes one import may produce, matching the plugin executor's
/// expansion cap for the same reason: a 40 KB zip that becomes 5 GB fills the
/// disk long before any path check becomes interesting.
pub const MAX_IMPORT_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Cap on directory depth walked while copying a folder.
pub const MAX_IMPORT_DEPTH: usize = 32;

/// Where the store assembles an import before it has an id.
const INCOMING: &str = ".incoming";

/// How a local mod got here.
///
/// Kept because it is the first thing anybody asks about a row they do not
/// recognise, and because the three have genuinely different repair stories: a
/// dropped archive can be re-dropped, an adopted folder is still in the game
/// directory, and one taken from another manager is still in that manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    /// A file the user dragged into the window.
    Dropped,
    /// A folder the user dragged into the window.
    Folder,
    /// Something already in the game directory that nothing claimed.
    Adopted,
    /// Read out of another mod manager's own storage.
    Manager,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dropped => "dropped",
            Self::Folder => "folder",
            Self::Adopted => "adopted",
            Self::Manager => "manager",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "folder" => Self::Folder,
            "adopted" => Self::Adopted,
            "manager" => Self::Manager,
            // An unknown value from a future schema reads as the most
            // conservative thing it could be, rather than failing the row.
            _ => Self::Dropped,
        }
    }
}

/// How the payload should be treated.
///
/// The one genuinely ambiguous decision in an import, so it is an explicit
/// choice rather than a heuristic buried in the copy: `.jar` is a zip and must
/// not be unpacked, a Minecraft resource pack is a `.zip` and must not be
/// unpacked either, and a mod distributed as a zip of loose files must be.
/// [`suggest_payload`] picks a default from the game's own rules; the UI shows
/// it and the user can flip it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Payload {
    /// Copy the file as it is. What a `.jar`, a `.dll`, a `.pak` or a resource
    /// pack needs.
    File,
    /// Unpack it. What a zip of loose files needs.
    Unpack,
    /// Copy the directory tree.
    Folder,
}

/// What one import produced, before it has a database id.
#[derive(Debug)]
pub struct Prepared {
    /// The assembled tree, under `<store>/.incoming/`. Moved into place by
    /// [`commit`], removed by [`discard`], and never left behind by either.
    pub dir: PathBuf,
    /// A name to show, from the sidecar or from the file name.
    pub name: String,
    /// Where the payload sits under the game folder.
    pub rel_path: String,
    /// The sidecar, when the archive had a readable one.
    pub metadata: Option<ModMetadata>,
    pub files: usize,
    pub bytes: u64,
    pub payload: Payload,
}

/// What the caller has to supply for a guess to be made.
pub struct ImportOptions<'a> {
    /// Where the game's `sandbox.json` says mods go, in declaration order.
    /// Empty for a game with no rules — the payload then lands at the game's
    /// root, which is the only remaining honest answer.
    pub mod_targets: &'a [String],
    /// File extensions the game's install rules accept, lower-cased and without
    /// the dot. What decides [`Payload::File`] against [`Payload::Unpack`].
    pub mod_extensions: &'a [String],
    /// An explicit choice, when the user has made one. `None` means "suggest".
    pub payload: Option<Payload>,
    /// An explicit destination, when the user has set one.
    pub rel_path: Option<String>,
}

/// A local mod's directory in the store.
pub fn local_root(store: &Path, id: i64) -> PathBuf {
    store.join(id.to_string())
}

/// Decide how to treat a dropped path, from the game's own rules.
///
/// A directory is always a directory. Otherwise the GAME decides: if its
/// install rules accept this extension, the file is a mod in the shape this
/// game wants and is copied verbatim — that is the whole of why a Minecraft
/// `.jar` and a Minecraft resource-pack `.zip` both survive an import intact.
/// Only a file no rule claims, and which is an archive this crate can open, is
/// unpacked.
pub fn suggest_payload(source: &Path, extensions: &[String]) -> Payload {
    if source.is_dir() {
        return Payload::Folder;
    }

    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let ext = name.rsplit('.').next().unwrap_or_default();

    if extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
        return Payload::File;
    }

    if is_unpackable(&name) {
        Payload::Unpack
    } else {
        Payload::File
    }
}

/// Archives this crate can open. Deliberately not `.jar`, `.7z` or `.rar`:
/// the first is a mod, and the other two need a decoder that is not a
/// dependency here.
fn is_unpackable(lower_name: &str) -> bool {
    lower_name.ends_with(".zip")
        || lower_name.ends_with(".tar")
        || lower_name.ends_with(".tar.gz")
        || lower_name.ends_with(".tgz")
}

/// Assemble one import into the store's staging area.
///
/// Nothing is registered and nothing is deployed — the caller mints an id, then
/// calls [`commit`] or [`discard`]. Splitting it that way is what stops a failed
/// database write leaving a directory nothing owns.
pub fn prepare(store: &Path, source: &Path, opts: &ImportOptions<'_>) -> AppResult<Prepared> {
    if !source.exists() {
        return Err(AppError::invalid(
            "That file is no longer where it was when it was dropped.",
        ));
    }

    let payload = opts
        .payload
        .unwrap_or_else(|| suggest_payload(source, opts.mod_extensions));

    let incoming = store.join(INCOMING);

    // Cleared rather than reused: a previous run interrupted mid-copy leaves a
    // partial tree, and a partial tree merged into this one is an import that
    // silently contains somebody else's files.
    let _ = std::fs::remove_dir_all(&incoming);
    std::fs::create_dir_all(&incoming)?;

    // The payload goes under a neutral subdirectory first, so the sidecar can
    // be read and the layout decided before anything is placed at its final
    // relative path.
    let raw = incoming.join("payload");
    std::fs::create_dir_all(&raw)?;

    let result = assemble(source, &raw, payload);

    let (files, bytes) = match result {
        Ok(counts) => counts,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&incoming);

            return Err(e);
        }
    };

    if files == 0 {
        let _ = std::fs::remove_dir_all(&incoming);

        return Err(AppError::invalid(
            "There was nothing in that — no files were imported.",
        ));
    }

    let metadata = read_sidecar(&raw);

    let rel_path = match &opts.rel_path {
        Some(explicit) => sanitise_rel(explicit)?,
        None => suggest_rel_path(&raw, metadata.as_ref(), opts.mod_targets),
    };

    // Now move the payload to where it belongs under the game folder.
    let root = incoming.join("root");
    let placed = if rel_path.is_empty() {
        root.clone()
    } else {
        join_relative(&root, &rel_path)?
    };

    if let Some(parent) = placed.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if let Err(e) = std::fs::rename(&raw, &placed) {
        let _ = std::fs::remove_dir_all(&incoming);

        return Err(AppError::internal(format!(
            "could not lay out the import: {e}"
        )));
    }

    // The sidecar is bookkeeping about the archive, not a file the game wants
    // in its folder. It has already been read.
    remove_sidecar(&placed);

    let name = metadata
        .as_ref()
        .and_then(|m| m.name.clone())
        .unwrap_or_else(|| display_name(source));

    Ok(Prepared {
        dir: root,
        name,
        rel_path,
        metadata,
        files,
        bytes,
        payload,
    })
}

/// Move a prepared import to its permanent home.
pub fn commit(store: &Path, prepared: &Prepared, id: i64) -> AppResult<()> {
    let target = local_root(store, id);

    // A leftover directory at this id means a previous row was deleted without
    // its files, which is recoverable and must not block the import.
    let _ = std::fs::remove_dir_all(&target);

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::rename(&prepared.dir, &target)
        .map_err(|e| AppError::internal(format!("could not store the import: {e}")))?;

    let _ = std::fs::remove_dir_all(store.join(INCOMING));

    Ok(())
}

/// Throw a prepared import away.
pub fn discard(store: &Path) {
    let _ = std::fs::remove_dir_all(store.join(INCOMING));
}

/// Delete one local mod's files.
pub fn remove(store: &Path, id: i64) {
    let _ = std::fs::remove_dir_all(local_root(store, id));
}

/// Move a local mod's payload to a different place under the game folder.
///
/// A rename inside the store, so changing "this goes in `mods`" to "this goes
/// in `BepInEx/plugins`" costs nothing and does not need the original archive —
/// which the user may well have deleted. The mod has to be re-deployed
/// afterwards, and the caller is what knows that.
pub fn relayout(store: &Path, id: i64, from: &str, to: &str) -> AppResult<()> {
    let from = sanitise_rel(from)?;
    let to = sanitise_rel(to)?;

    if from == to {
        return Ok(());
    }

    let root = local_root(store, id);

    if !root.is_dir() {
        return Err(AppError::invalid(
            "This mod's files are missing — import it again.",
        ));
    }

    let current = if from.is_empty() {
        root.clone()
    } else {
        join_relative(&root, &from)?
    };

    if !current.is_dir() {
        return Err(AppError::invalid(
            "This mod's files are not where its install path says they are.",
        ));
    }

    /*
     * Via a sibling temporary directory rather than in place. Moving `root` to
     * `root/mods` is a rename of a directory into itself, which every platform
     * refuses — and the empty-to-non-empty case is exactly the one a user hits
     * first, because "the game's root" is what an unrecognised archive gets.
     */
    let holding = root.with_extension("relayout");

    let _ = std::fs::remove_dir_all(&holding);

    std::fs::rename(&current, &holding)
        .map_err(|e| AppError::internal(format!("could not move the files: {e}")))?;

    // Whatever else was under the old root was empty directories on the way to
    // the payload; the payload itself is now in `holding`.
    let _ = std::fs::remove_dir_all(&root);

    let placed = if to.is_empty() {
        root.clone()
    } else {
        join_relative(&root, &to)?
    };

    if let Some(parent) = placed.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if let Err(e) = std::fs::rename(&holding, &placed) {
        // Put it back where it was rather than leaving the mod with no files.
        let _ = std::fs::create_dir_all(&root);
        let _ = std::fs::rename(&holding, &current);

        return Err(AppError::internal(format!("could not move the files: {e}")));
    }

    Ok(())
}

/// Count what is actually on disk for one local mod.
///
/// Read back rather than trusted from the row, because the row records what an
/// import produced and the store is an ordinary directory somebody may have
/// emptied.
pub fn measure(store: &Path, id: i64) -> (usize, u64) {
    let mut files = 0usize;
    let mut bytes = 0u64;

    walk(&local_root(store, id), 0, &mut |_, meta| {
        files += 1;
        bytes += meta.len();

        files < MAX_IMPORT_FILES
    });

    (files, bytes)
}

// ------------------------------------------------------------------ Assembly

/// Put the source's contents under `dest`, by whichever mechanism was chosen.
fn assemble(source: &Path, dest: &Path, payload: Payload) -> AppResult<(usize, u64)> {
    match payload {
        Payload::Unpack => {
            let written = crate::plugins::steps::extract_archive(source, dest, 0, &[])?;

            let mut bytes = 0u64;
            walk(dest, 0, &mut |_, meta| {
                bytes += meta.len();
                true
            });

            Ok((written.len(), bytes))
        }
        Payload::File => {
            let name = source
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| AppError::invalid("That file has no usable name."))?;

            // Through `join_relative` like any other untrusted name: a dropped
            // file's name is whatever the filesystem it came from allowed, and
            // this one is about to become a path component.
            let target = join_relative(dest, name)?;

            let bytes = std::fs::copy(source, &target)?;

            Ok((1, bytes))
        }
        Payload::Folder => copy_tree(source, dest),
    }
}

/// Copy a directory tree, bounded on every axis a real machine can exceed.
///
/// Symlinks are SKIPPED rather than followed, for the reason
/// [`crate::detect::walk`] gives: `~/.wine/dosdevices/z: -> /` is a layout
/// people actually have, and following one turns "import this mod folder" into
/// "copy the filesystem".
fn copy_tree(source: &Path, dest: &Path) -> AppResult<(usize, u64)> {
    fn inner(
        source: &Path,
        dest: &Path,
        depth: usize,
        files: &mut usize,
        bytes: &mut u64,
    ) -> AppResult<()> {
        if depth > MAX_IMPORT_DEPTH {
            return Err(AppError::invalid(
                "That folder is nested more deeply than the app will copy.",
            ));
        }

        std::fs::create_dir_all(dest)?;

        for entry in std::fs::read_dir(source)?.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }

            let Ok(meta) = entry.metadata() else { continue };

            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };

            let target = join_relative(dest, &name)?;

            if meta.is_dir() {
                inner(&entry.path(), &target, depth + 1, files, bytes)?;

                continue;
            }

            if !meta.is_file() {
                continue;
            }

            *files += 1;
            *bytes += meta.len();

            if *files > MAX_IMPORT_FILES {
                return Err(AppError::invalid("That folder holds too many files."));
            }

            if *bytes > MAX_IMPORT_BYTES {
                return Err(AppError::invalid("That folder is too large to import."));
            }

            std::fs::copy(entry.path(), &target)?;
        }

        Ok(())
    }

    let mut files = 0usize;
    let mut bytes = 0u64;

    inner(source, dest, 0, &mut files, &mut bytes)?;

    Ok((files, bytes))
}

/// Visit every regular file under `dir`. Stops when the callback returns false.
fn walk(dir: &Path, depth: usize, visit: &mut impl FnMut(&Path, &std::fs::Metadata) -> bool) {
    if depth > MAX_IMPORT_DEPTH {
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
            walk(&entry.path(), depth + 1, visit);
        } else if meta.is_file() && !visit(&entry.path(), &meta) {
            return;
        }
    }
}

// ------------------------------------------------------------------- Layout

/// Read the archive's own description of itself, if it left one.
fn read_sidecar(root: &Path) -> Option<ModMetadata> {
    for candidate in [METADATA_FILE, METADATA_FILE_ALT] {
        let path = join_relative(root, candidate).ok()?;

        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };

        if !meta.is_file() {
            continue;
        }

        if let Ok(raw) = std::fs::read(&path) {
            if let Some(parsed) = ModMetadata::parse(&raw) {
                return Some(parsed);
            }
        }
    }

    None
}

fn remove_sidecar(root: &Path) {
    for candidate in [METADATA_FILE, METADATA_FILE_ALT] {
        if let Ok(path) = join_relative(root, candidate) {
            let _ = std::fs::remove_file(&path);
        }
    }

    // `.tmc/` is left only if it is now empty; a bundle that put other things
    // in there keeps them.
    if let Ok(dir) = join_relative(root, ".tmc") {
        let _ = std::fs::remove_dir(dir);
    }
}

/// Where this payload most likely belongs under the game folder.
///
/// See the module header for the order and why it is that order. The result is
/// a suggestion the user can overrule, which is why nothing here tries harder.
fn suggest_rel_path(payload: &Path, metadata: Option<&ModMetadata>, targets: &[String]) -> String {
    if let Some(declared) = metadata.and_then(|m| m.install_path.as_deref()) {
        if let Ok(clean) = sanitise_rel(declared) {
            return clean;
        }
    }

    // Does the payload already look like a slice of the game folder? It does if
    // its top level holds a directory the game declares as a mod target — an
    // archive containing `mods/` for a game whose mods live in `mods` is
    // game-root-relative and must not be nested inside itself.
    let tops = top_level_dirs(payload);

    for target in targets {
        let head = target.split('/').next().unwrap_or(target);

        if head.is_empty() {
            continue;
        }

        if tops.iter().any(|t| t.eq_ignore_ascii_case(head)) {
            return String::new();
        }
    }

    targets
        .iter()
        .find_map(|t| sanitise_rel(t).ok())
        .unwrap_or_default()
}

fn top_level_dirs(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter(|e| e.metadata().is_ok_and(|m| m.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .take(256)
        .collect()
}

/// Normalise an install path: `/`-separated, no traversal, no leading slash.
///
/// Through [`join_relative`] against a throwaway base rather than by hand,
/// because that function is where every rule about a Windows trailing dot, an
/// alternate data stream and a drive prefix already lives — and a second
/// implementation of those rules is a second one to get wrong.
pub fn sanitise_rel(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim().trim_matches('/').trim_matches('\\');

    if trimmed.is_empty() {
        return Ok(String::new());
    }

    let base = Path::new("/tmc-check");
    let joined = join_relative(base, trimmed)?;

    let rel = joined
        .strip_prefix(base)
        .map_err(|_| AppError::jail("Install path must be relative."))?;

    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();

    if parts.len() > MAX_IMPORT_DEPTH {
        return Err(AppError::jail("Install path is nested too deeply."));
    }

    Ok(parts.join("/"))
}

/// A readable name from a path, with an archive extension taken off.
fn display_name(source: &Path) -> String {
    let raw = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Imported mod");

    let lower = raw.to_ascii_lowercase();

    let stem = if lower.ends_with(".tar.gz") {
        &raw[..raw.len() - 7]
    } else if is_unpackable(&lower) {
        raw.rsplit_once('.').map(|(head, _)| head).unwrap_or(raw)
    } else {
        raw
    };

    let cleaned: String = stem.chars().filter(|c| !c.is_control()).take(200).collect();

    if cleaned.trim().is_empty() {
        "Imported mod".to_string()
    } else {
        cleaned.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn opts<'a>(targets: &'a [String], exts: &'a [String]) -> ImportOptions<'a> {
        ImportOptions {
            mod_targets: targets,
            mod_extensions: exts,
            payload: None,
            rel_path: None,
        }
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }

        std::fs::write(path, body).expect("write");
    }

    fn zip_of(entries: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write as _;

        let mut buf = std::io::Cursor::new(Vec::new());

        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();

            for (name, body) in entries {
                writer.start_file(*name, options).expect("entry");
                writer.write_all(body.as_bytes()).expect("body");
            }

            writer.finish().expect("finish");
        }

        buf.into_inner()
    }

    /// The single most common drop: a jar for a game whose rules accept jars.
    /// It must land verbatim in `mods`, not be unpacked as the zip it is.
    #[test]
    fn a_jar_is_a_file_not_an_archive() {
        let dir = store();
        let src = dir.path().join("cool-mod-1.4.jar");

        write(&src, "PK-not-really");

        let targets = vec!["mods".to_string()];
        let exts = vec!["jar".to_string()];

        let prepared = prepare(dir.path(), &src, &opts(&targets, &exts)).expect("prepare");

        assert_eq!(prepared.payload, Payload::File);
        assert_eq!(prepared.rel_path, "mods");
        assert_eq!(prepared.name, "cool-mod-1.4.jar");
        assert!(prepared.dir.join("mods/cool-mod-1.4.jar").is_file());
    }

    #[test]
    fn a_zip_of_loose_files_is_unpacked_under_the_mod_target() {
        let dir = store();
        let src = dir.path().join("pack.zip");

        std::fs::write(&src, zip_of(&[("a.txt", "one"), ("b/c.txt", "two")])).expect("write");

        let targets = vec!["BepInEx/plugins".to_string()];

        let prepared = prepare(dir.path(), &src, &opts(&targets, &[])).expect("prepare");

        assert_eq!(prepared.payload, Payload::Unpack);
        assert_eq!(prepared.rel_path, "BepInEx/plugins");
        assert!(prepared.dir.join("BepInEx/plugins/a.txt").is_file());
        assert!(prepared.dir.join("BepInEx/plugins/b/c.txt").is_file());
    }

    /// The case the nesting rule exists for: an archive that already holds
    /// `mods/` is a slice of the game folder and must not become `mods/mods/`.
    #[test]
    fn an_archive_that_is_already_game_relative_is_not_nested_again() {
        let dir = store();
        let src = dir.path().join("modpack.zip");

        std::fs::write(&src, zip_of(&[("mods/a.jar", "x"), ("config/b.cfg", "y")])).expect("write");

        let targets = vec!["mods".to_string()];

        let prepared = prepare(dir.path(), &src, &opts(&targets, &[])).expect("prepare");

        assert_eq!(prepared.rel_path, "");
        assert!(prepared.dir.join("mods/a.jar").is_file());
        assert!(prepared.dir.join("config/b.cfg").is_file());
    }

    #[test]
    fn a_sidecar_names_the_mod_and_where_it_goes() {
        let dir = store();
        let src = dir.path().join("something.zip");

        let sidecar = serde_json::json!({
            "metadataVersion": 1,
            "source": "https://moddingcommunity.com",
            "kind": "mod",
            "itemId": 42,
            "name": "Properly Named",
            "version": "2.0",
            "installPath": "BepInEx/plugins"
        })
        .to_string();

        std::fs::write(
            &src,
            zip_of(&[("tmc.json", sidecar.as_str()), ("thing.dll", "x")]),
        )
        .expect("write");

        let targets = vec!["mods".to_string()];

        let prepared = prepare(dir.path(), &src, &opts(&targets, &[])).expect("prepare");

        assert_eq!(prepared.name, "Properly Named");
        assert_eq!(prepared.rel_path, "BepInEx/plugins");
        assert_eq!(prepared.metadata.as_ref().and_then(|m| m.item_id), Some(42));

        // The sidecar is bookkeeping, not a file the game wants.
        assert!(!prepared.dir.join("BepInEx/plugins/tmc.json").exists());
        assert!(prepared.dir.join("BepInEx/plugins/thing.dll").is_file());
    }

    /// A sidecar is a file from a stranger. It may name the mod; it may not
    /// name a path outside the store.
    #[test]
    fn a_sidecar_cannot_name_a_path_outside_the_store() {
        let dir = store();
        let src = dir.path().join("evil.zip");

        let sidecar = serde_json::json!({
            "metadataVersion": 1,
            "installPath": "../../../../etc"
        })
        .to_string();

        std::fs::write(
            &src,
            zip_of(&[("tmc.json", sidecar.as_str()), ("thing.dll", "x")]),
        )
        .expect("write");

        let targets = vec!["mods".to_string()];

        let prepared = prepare(dir.path(), &src, &opts(&targets, &[])).expect("prepare");

        // Refused, so the game's own target is used instead.
        assert_eq!(prepared.rel_path, "mods");
        assert!(prepared.dir.join("mods/thing.dll").is_file());
    }

    #[test]
    fn a_folder_is_copied_and_symlinks_are_not_followed() {
        let dir = store();
        let src = dir.path().join("MyMod");

        write(&src.join("a.txt"), "one");
        write(&src.join("nested/b.txt"), "two");

        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc", src.join("escape")).expect("symlink");

        let targets = vec!["Mods".to_string()];

        let prepared = prepare(dir.path(), &src, &opts(&targets, &[])).expect("prepare");

        assert_eq!(prepared.payload, Payload::Folder);
        assert_eq!(prepared.files, 2);
        assert!(prepared.dir.join("Mods/a.txt").is_file());
        assert!(prepared.dir.join("Mods/nested/b.txt").is_file());
        assert!(!prepared.dir.join("Mods/escape").exists());
    }

    #[test]
    fn committing_moves_the_payload_and_clears_the_staging_area() {
        let dir = store();
        let src = dir.path().join("thing.jar");

        write(&src, "x");

        let targets = vec!["mods".to_string()];
        let exts = vec!["jar".to_string()];

        let prepared = prepare(dir.path(), &src, &opts(&targets, &exts)).expect("prepare");

        commit(dir.path(), &prepared, 7).expect("commit");

        assert!(local_root(dir.path(), 7).join("mods/thing.jar").is_file());
        assert!(!dir.path().join(INCOMING).exists());
    }

    /// The two directions that are easy to get wrong, because one of them is a
    /// rename of a directory into itself.
    #[test]
    fn relayout_moves_between_the_root_and_a_subdirectory_both_ways() {
        let dir = store();
        let root = local_root(dir.path(), 3);

        write(&root.join("a.jar"), "x");

        relayout(dir.path(), 3, "", "mods").expect("into mods");

        assert!(root.join("mods/a.jar").is_file());
        assert!(!root.join("a.jar").exists());

        relayout(dir.path(), 3, "mods", "BepInEx/plugins").expect("deeper");

        assert!(root.join("BepInEx/plugins/a.jar").is_file());

        relayout(dir.path(), 3, "BepInEx/plugins", "").expect("back to root");

        assert!(root.join("a.jar").is_file());
        assert!(!root.join("BepInEx").exists());
    }

    #[test]
    fn an_install_path_cannot_escape_the_store() {
        assert!(sanitise_rel("../up").is_err());
        assert!(sanitise_rel("/absolute").is_ok_and(|p| p == "absolute"));
        assert_eq!(sanitise_rel("  mods/  ").expect("ok"), "mods");
        assert_eq!(sanitise_rel("").expect("ok"), "");
    }

    #[test]
    fn an_empty_import_is_refused_rather_than_stored() {
        let dir = store();
        let src = dir.path().join("Empty");

        std::fs::create_dir_all(&src).expect("mkdir");

        let err = prepare(dir.path(), &src, &opts(&[], &[])).expect_err("refused");

        assert!(err.to_string().contains("nothing"));
        assert!(!dir.path().join(INCOMING).exists());
    }
}
