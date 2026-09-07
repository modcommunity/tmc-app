//! The virtual tree: what the game sees when it looks at its own folder.
//!
//! This is the whole brain of the virtual-filesystem strategy, and it is
//! deliberately platform-independent — a lookup table from a path the game asks
//! for to a real path on disk, plus the directory listings that make the
//! redirected files *discoverable*.
//!
//! WHY LISTINGS ARE HALF THE WORK
//! ------------------------------
//! Redirecting `CreateFile` alone is not enough and it is the mistake every
//! first attempt makes. A mod loader does not open `mods/cool.jar` by name — it
//! *enumerates* `mods/` and opens whatever it finds. So the tree has to answer
//! two questions:
//!
//!   * **"open this exact path"** — resolve one virtual path to one real file;
//!   * **"what is in this directory"** — merge the real directory's own entries
//!     with every virtual entry underneath it, with the virtual ones winning.
//!
//! A tree that only did the first would make a modded game look completely
//! unmodded while every individual file was, technically, redirectable.
//!
//! CASE
//! ----
//! Windows paths are case-insensitive, and a game that writes `Mods\Cool.Jar`
//! must find a file staged as `mods/cool.jar`. Every key is therefore lowercased
//! and `\` is normalised to `/`. The stored value keeps its original case,
//! because that is what gets opened and Windows is the only platform this runs
//! on where the difference does not matter.
//!
//! WHAT IS NOT HERE
//! ----------------
//! Writes. A game that creates a file inside a virtualised directory writes it
//! to the REAL directory, unredirected — see the module header on
//! [`crate::hooks`] for why that is the right call rather than a gap.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Cap on entries in one tree. A large Skyrim list is tens of thousands of
/// loose files; this has room and is still bounded.
pub const MAX_ENTRIES: usize = 250_000;

/// Cap on one path. Windows' own limit is 32,767 with the `\\?\` prefix.
pub const MAX_PATH_LEN: usize = 4096;

/// One virtual file: a path the game may ask for, and the real file behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    /// Relative to the game directory, `/`-separated, original case.
    pub virtual_path: String,
    /// Absolute, on disk, original case.
    pub real_path: String,
}

/// The merged view of one game directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VirtualTree {
    /// The game directory itself, absolute. Every virtual path is relative to
    /// this and a path outside it is not virtualised at all.
    pub root: String,

    /// Lowercased virtual path → the real file.
    ///
    /// A `BTreeMap` rather than a hash map: the serialised form has to be
    /// deterministic so that two identical trees produce identical bytes, which
    /// is what lets the injected process notice a change by comparing a
    /// revision rather than diffing.
    files: BTreeMap<String, Mapping>,

    /// Lowercased directory path → the child NAMES it gains, original case.
    ///
    /// Derived from `files` and stored rather than recomputed, because the
    /// injected process answers `FindFirstFile` from it on a hot path and
    /// walking every mapping per call would be quadratic in the worst case a
    /// mod loader routinely produces.
    dirs: BTreeMap<String, BTreeSet<String>>,
}

impl VirtualTree {
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            root: normalise_separators(&root.into()),
            files: BTreeMap::new(),
            dirs: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Add one file. A later insert for the same path wins, which is how
    /// priority is expressed — the caller adds in ascending priority order.
    pub fn insert(&mut self, virtual_path: &str, real_path: &str) -> bool {
        let Some(clean) = clean_relative(virtual_path) else {
            return false;
        };

        if real_path.len() > MAX_PATH_LEN || real_path.is_empty() {
            return false;
        }

        let key = clean.to_ascii_lowercase();

        /*
         * The cap counts NEW paths only. Refusing a replacement at the cap
         * would drop an override rather than an addition — the caller inserts
         * in ascending priority order, so the entry being refused is the
         * higher-priority one, and the tree would quietly resolve to the file
         * the user's load order says loses.
         */
        if !self.files.contains_key(&key) && self.files.len() >= MAX_ENTRIES {
            return false;
        }

        // Every ancestor directory gains this entry's next component, so a
        // listing of `mods` finds `cool.jar` and a listing of the root finds
        // `mods`.
        let parts: Vec<&str> = clean.split('/').collect();

        for depth in 0..parts.len() {
            let dir = parts[..depth].join("/");
            let child = parts[depth].to_string();

            self.dirs
                .entry(dir.to_ascii_lowercase())
                .or_default()
                .insert(child);
        }

        self.files.insert(
            key,
            Mapping {
                virtual_path: clean,
                real_path: normalise_separators(real_path),
            },
        );

        true
    }

    /// The real file behind a virtual path, or `None`.
    ///
    /// Takes an ABSOLUTE path as the game would pass it, and answers `None` for
    /// anything outside the root — a game opens hundreds of files that have
    /// nothing to do with its own directory, and every one of them has to fall
    /// through untouched and fast.
    pub fn resolve_absolute(&self, path: &str) -> Option<&str> {
        let relative = self.relative_of(path)?;

        self.files
            .get(&relative.to_ascii_lowercase())
            .map(|m| m.real_path.as_str())
    }

    /// The real file behind a path already known to be relative to the root.
    pub fn resolve(&self, relative: &str) -> Option<&str> {
        let clean = clean_relative(relative)?;

        self.files
            .get(&clean.to_ascii_lowercase())
            .map(|m| m.real_path.as_str())
    }

    /// The names a directory gains, for a merged listing.
    ///
    /// Empty for a directory the tree adds nothing to, which is the answer for
    /// almost every directory a game enumerates.
    pub fn extra_entries(&self, relative_dir: &str) -> &BTreeSet<String> {
        static EMPTY: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();

        let key = clean_relative_dir(relative_dir).to_ascii_lowercase();

        self.dirs
            .get(&key)
            .unwrap_or_else(|| EMPTY.get_or_init(BTreeSet::new))
    }

    /// The same, from an absolute path.
    pub fn extra_entries_absolute(&self, dir: &str) -> &BTreeSet<String> {
        static EMPTY: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();

        match self.relative_of(dir) {
            Some(relative) => self.extra_entries(&relative),
            None => EMPTY.get_or_init(BTreeSet::new),
        }
    }

    /// Does the tree add anything at or under this absolute directory?
    ///
    /// The cheap guard a hook uses before doing anything expensive.
    pub fn touches(&self, dir: &str) -> bool {
        self.relative_of(dir).is_some_and(|rel| {
            self.dirs
                .contains_key(&clean_relative_dir(&rel).to_ascii_lowercase())
        })
    }

    /// Every mapping, for tests and for the report.
    pub fn mappings(&self) -> impl Iterator<Item = &Mapping> {
        self.files.values()
    }

    /// An absolute path expressed relative to the root, or `None` when it is
    /// outside.
    fn relative_of(&self, path: &str) -> Option<String> {
        let path = normalise_separators(path);

        // `\\?\C:\Games\X` and `C:\Games\X` are the same file, and a game
        // handed the first by its own launcher would otherwise miss every
        // redirect.
        let path = path
            .strip_prefix("//?/UNC/")
            .map(|rest| format!("//{rest}"))
            .unwrap_or_else(|| {
                path.strip_prefix("//?/")
                    .map(str::to_string)
                    .unwrap_or(path)
            });

        let lower = path.to_ascii_lowercase();
        let root = self.root.to_ascii_lowercase();

        let rest = lower.strip_prefix(&root)?;

        // The root itself, or a path under it — but not a sibling whose name
        // merely starts with the root's (`C:/Games/X2` under `C:/Games/X`).
        let rest = match rest.chars().next() {
            None => "",
            Some('/') => &rest[1..],
            Some(_) => return None,
        };

        // Take the ORIGINAL case back: the value is what gets opened.
        let start = path.len() - rest.len();

        Some(path[start..].to_string())
    }
}

/// `\` → `/`, and collapse repeats.
///
/// Repeats matter: `C:\Games\\X` is a path Windows accepts and a naive prefix
/// comparison rejects.
fn normalise_separators(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut last_sep = false;

    for ch in path.chars() {
        let sep = ch == '\\' || ch == '/';

        if sep {
            // A leading `//` is a UNC prefix and is kept; everything else
            // collapses.
            if last_sep && !out.is_empty() && out.len() > 1 {
                continue;
            }

            out.push('/');
            last_sep = true;
        } else {
            out.push(ch);
            last_sep = false;
        }
    }

    // A trailing separator makes `mods/` and `mods` different keys.
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }

    out
}

/// A relative virtual path, or `None` if it is not one.
///
/// Refuses `..`, absolutes and drive prefixes for the same reason the plugin
/// jail does: this string is joined onto a directory and handed to the
/// operating system.
fn clean_relative(path: &str) -> Option<String> {
    if path.is_empty() || path.len() > MAX_PATH_LEN || path.contains('\0') {
        return None;
    }

    let normalised = normalise_separators(path);

    // An absolute path, a drive letter, or a UNC root.
    if normalised.starts_with('/') || normalised.chars().nth(1) == Some(':') {
        return None;
    }

    let mut parts: Vec<&str> = Vec::new();

    for part in normalised.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            other => parts.push(other),
        }
    }

    if parts.is_empty() {
        return None;
    }

    Some(parts.join("/"))
}

/// The same, for a directory — where the empty string is the root and legal.
fn clean_relative_dir(path: &str) -> String {
    clean_relative(path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> VirtualTree {
        let mut tree = VirtualTree::new("C:/Games/Skyrim");

        tree.insert(
            "Data/textures/sky.dds",
            "D:/staging/a/Data/textures/sky.dds",
        );
        tree.insert("Data/mod.esp", "D:/staging/a/Data/mod.esp");

        tree
    }

    #[test]
    fn a_mapped_path_resolves_to_its_real_file() {
        let tree = tree();

        assert_eq!(
            tree.resolve("Data/mod.esp"),
            Some("D:/staging/a/Data/mod.esp")
        );

        assert_eq!(
            tree.resolve_absolute("C:/Games/Skyrim/Data/mod.esp"),
            Some("D:/staging/a/Data/mod.esp")
        );
    }

    /// Windows paths are case-insensitive, and a game that writes
    /// `Data\Mod.ESP` must find a file staged as `Data/mod.esp`.
    #[test]
    fn lookups_ignore_case_and_separator() {
        let tree = tree();

        for asked in [
            "C:/Games/Skyrim/Data/mod.esp",
            "C:\\Games\\Skyrim\\Data\\Mod.ESP",
            "c:/games/skyrim/data/MOD.esp",
            "C:\\Games\\Skyrim\\\\Data\\mod.esp",
        ] {
            assert_eq!(
                tree.resolve_absolute(asked),
                Some("D:/staging/a/Data/mod.esp"),
                "{asked}"
            );
        }
    }

    /// A launcher that hands the game an extended-length path would otherwise
    /// miss every redirect.
    #[test]
    fn an_extended_length_prefix_is_understood() {
        let tree = tree();

        assert_eq!(
            tree.resolve_absolute("\\\\?\\C:\\Games\\Skyrim\\Data\\mod.esp"),
            Some("D:/staging/a/Data/mod.esp")
        );
    }

    /// A game opens hundreds of files that have nothing to do with its own
    /// folder. Every one has to fall through untouched.
    #[test]
    fn a_path_outside_the_root_is_not_virtualised() {
        let tree = tree();

        for outside in [
            "C:/Windows/System32/kernel32.dll",
            "D:/staging/a/Data/mod.esp",
            // The dangerous one: a sibling whose name starts with the root's.
            "C:/Games/Skyrim2/Data/mod.esp",
            "C:/Games/Skyrim.bak/Data/mod.esp",
        ] {
            assert_eq!(tree.resolve_absolute(outside), None, "{outside}");
        }
    }

    /// Redirecting `CreateFile` alone makes a modded game look unmodded: a mod
    /// loader ENUMERATES its directory rather than opening files by name.
    #[test]
    fn every_ancestor_directory_gains_its_child() {
        let tree = tree();

        assert!(tree.extra_entries("").contains("Data"));
        assert!(tree.extra_entries("Data").contains("mod.esp"));
        assert!(tree.extra_entries("Data").contains("textures"));
        assert!(tree.extra_entries("Data/textures").contains("sky.dds"));

        // And by the case the game happens to use.
        assert!(tree.extra_entries("DATA").contains("mod.esp"));

        // A directory the tree adds nothing to.
        assert!(tree.extra_entries("Data/meshes").is_empty());
    }

    #[test]
    fn listings_work_from_an_absolute_directory_too() {
        let tree = tree();

        assert!(tree
            .extra_entries_absolute("C:\\Games\\Skyrim\\Data")
            .contains("mod.esp"));

        assert!(tree
            .extra_entries_absolute("C:/Windows/System32")
            .is_empty());
    }

    #[test]
    fn touches_is_the_cheap_guard_a_hook_needs() {
        let tree = tree();

        assert!(tree.touches("C:/Games/Skyrim"));
        assert!(tree.touches("C:/Games/Skyrim/Data"));
        assert!(!tree.touches("C:/Games/Skyrim/Data/meshes"));
        assert!(!tree.touches("C:/Windows"));
    }

    /// The caller adds in ascending priority order, so a later insert wins —
    /// which is how the merge tree's own priority reaches this one.
    #[test]
    fn a_later_insert_wins_the_same_path() {
        let mut tree = VirtualTree::new("C:/Games/X");

        tree.insert("Data/sky.dds", "D:/low/sky.dds");
        tree.insert("Data/sky.dds", "D:/high/sky.dds");

        assert_eq!(tree.resolve("Data/sky.dds"), Some("D:/high/sky.dds"));
        assert_eq!(tree.len(), 1);
    }

    /// The string is joined onto a directory and handed to the OS, so it gets
    /// the same refusals the plugin jail applies.
    #[test]
    fn an_unsafe_virtual_path_is_refused() {
        let mut tree = VirtualTree::new("C:/Games/X");

        for bad in [
            "../escape.dll",
            "Data/../../escape.dll",
            "/absolute",
            "C:/absolute",
            "\\\\server\\share\\x",
            "",
            "..",
        ] {
            assert!(!tree.insert(bad, "D:/staging/x"), "{bad} should be refused");
        }

        assert!(tree.is_empty());
    }

    #[test]
    fn a_tree_is_bounded() {
        let mut tree = VirtualTree::new("C:/Games/X");

        assert!(tree.insert("a.txt", "D:/x/a.txt"));

        let long = "x".repeat(MAX_PATH_LEN + 1);

        assert!(
            !tree.insert("b.txt", &long),
            "an absurd real path is refused"
        );
        assert!(
            !tree.insert(&long, "D:/x/b.txt"),
            "an absurd virtual path is refused"
        );

        assert_eq!(tree.len(), 1, "neither refusal left a partial entry");
    }

    /// The entry cap, exercised for real. Slow enough to be worth saying why it
    /// is here: the blob is mapped into somebody else's game, so "the tree grew
    /// until the process died" has to be impossible rather than unlikely.
    #[test]
    fn the_entry_cap_is_enforced() {
        let mut tree = VirtualTree::new("C:/Games/X");

        for n in 0..MAX_ENTRIES {
            assert!(tree.insert(&format!("d/f{n}"), "D:/x/f"), "entry {n}");
        }

        assert_eq!(tree.len(), MAX_ENTRIES);
        assert!(!tree.insert("d/one-too-many", "D:/x/f"));

        // A path already present is a REPLACEMENT, not a new entry, so it is
        // still accepted at the cap — otherwise a full tree could never be
        // corrected.
        assert!(tree.insert("d/f0", "D:/y/f"));
        assert_eq!(tree.resolve("d/f0"), Some("D:/y/f"));
    }

    #[test]
    fn separators_normalise_without_eating_a_unc_prefix() {
        assert_eq!(normalise_separators("C:\\Games\\X"), "C:/Games/X");
        assert_eq!(normalise_separators("C:\\Games\\\\X\\"), "C:/Games/X");
        assert_eq!(normalise_separators("//server/share"), "//server/share");
        assert_eq!(normalise_separators("mods/"), "mods");
    }

    /// The serialised form has to be deterministic, so the injected process can
    /// notice a change by comparing a revision rather than diffing a tree.
    #[test]
    fn two_identical_trees_serialise_identically() {
        let mut a = VirtualTree::new("C:/Games/X");
        let mut b = VirtualTree::new("C:/Games/X");

        // Inserted in different orders.
        a.insert("z.txt", "D:/x/z.txt");
        a.insert("a.txt", "D:/x/a.txt");

        b.insert("a.txt", "D:/x/a.txt");
        b.insert("z.txt", "D:/x/z.txt");

        assert_eq!(
            serde_json::to_string(&a).expect("a"),
            serde_json::to_string(&b).expect("b")
        );
    }
}
