//! The virtual merge tree: what the game directory would look like if every
//! enabled mod in a sandbox were applied at once.
//!
//! This is the step every mod manager has and every one of them names
//! differently — MO2 calls it the virtual file tree, Vortex calls it the
//! deployment graph. It is the same idea in all of them: mods overlap, one of
//! them has to win each overlapping path, and the winner has to be decided
//! ONCE, in memory, before anything touches the disk.
//!
//! Deciding it up front rather than as a side effect of copy order is what
//! makes three things possible that are otherwise not:
//!
//!   * **A conflict report.** "Better Textures and HD Overhaul both provide
//!     `textures/sky.dds`; HD Overhaul wins" is a sentence the user can act on.
//!     Discovering it by watching one copy clobber another produces no sentence
//!     at all.
//!   * **Reordering without redeploying everything.** The tree is cheap; only
//!     the paths whose winner CHANGED need touching.
//!   * **A dry run.** The tree is the whole plan, so it can be shown before it
//!     is applied.
//!
//! PRIORITY
//! --------
//! Higher number wins, and the list arrives already ordered. Ties are broken by
//! the mod key so the answer is stable across runs — an unstable tiebreak means
//! a deploy that changes files for no reason, which on a linking strategy is
//! merely wasteful and on the direct strategy is a needless backup churn.
//!
//! WHAT IS DELIBERATELY NOT HERE
//! -----------------------------
//! Merging file CONTENTS. Two mods that both ship `config.json` are a conflict
//! to be reported, not a three-way merge to be attempted: nothing here
//! understands any game's file formats, and a manager that silently produces a
//! file neither author wrote is how "it works for me" bug reports are made.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};

/// Cap on the files one sandbox may deploy.
///
/// A large Skyrim list is tens of thousands of loose files, so this has room —
/// but it is not unbounded, because the walk below is the one place a symlink
/// loop or a pathological archive turns into an unresponsive app.
pub const MAX_FILES: usize = 250_000;

/// Cap on directory depth inside one mod's staging folder.
pub const MAX_DEPTH: usize = 32;

/// One mod as the merge tree sees it: a name, a priority and a folder of files.
#[derive(Debug, Clone)]
pub struct DeployMod {
    /// Stable identity — `mod:1234`, `asset:99`. Recorded in the ledger, so it
    /// has to survive a rename of the item.
    pub key: String,
    /// For the conflict report and the log. Not an identity.
    pub name: String,
    /// The folder holding this mod's files, laid out exactly as they should
    /// appear under the game directory.
    pub root: PathBuf,
    /// Higher wins.
    pub priority: i64,
}

/// The file that will be at `rel`, and where it comes from.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Winner {
    pub mod_key: String,
    pub mod_name: String,
    #[serde(skip)]
    pub source: PathBuf,
    pub size: u64,
}

/// One path more than one mod provides.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// `/`-separated, relative to the game directory.
    pub path: String,
    pub winner: String,
    /// Every other mod providing this path, in priority order.
    pub losers: Vec<String>,
}

/// The whole plan.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeTree {
    /// `rel` → the file that wins it. A `BTreeMap` so iteration is sorted,
    /// which makes a deploy create parent directories before their children
    /// and makes two runs over the same input produce identical logs.
    #[serde(skip)]
    pub files: BTreeMap<String, Winner>,

    pub conflicts: Vec<Conflict>,

    /// Mods that contributed no files at all. Almost always a mis-authored
    /// install rule, and invisible without saying so — the mod appears in the
    /// list, appears enabled, and does nothing.
    pub empty: Vec<String>,

    /// Files skipped and why, bounded so a broken staging folder cannot
    /// produce a million-line report.
    pub skipped: Vec<String>,
}

impl MergeTree {
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

/// How many skip reasons to keep. Past this the report says "and N more".
const MAX_SKIPPED: usize = 64;

/// Build the tree from mods in ASCENDING priority order.
///
/// Sorting happens here rather than being demanded of the caller: the caller
/// has the list in whatever order the database returned it, and a merge tree
/// built from an unsorted list is wrong in a way that produces a plausible
/// result — the last mod added wins instead of the highest priority, which
/// looks correct until somebody reorders their list and nothing changes.
pub fn build(mods: &[DeployMod]) -> AppResult<MergeTree> {
    let mut ordered: Vec<&DeployMod> = mods.iter().collect();

    ordered.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.key.cmp(&b.key)));

    let mut tree = MergeTree::default();
    let mut providers: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for entry in ordered {
        let before = tree.files.len();
        let mut contributed = 0usize;

        walk(
            &entry.root,
            &entry.root,
            0,
            &mut tree,
            entry,
            &mut providers,
            &mut contributed,
        )?;

        if contributed == 0 && before == tree.files.len() {
            tree.empty.push(entry.name.clone());
        }
    }

    // A path with more than one provider is a conflict; the LAST provider is
    // the winner, since the walk ran in ascending priority order.
    for (path, mut names) in providers {
        if names.len() < 2 {
            continue;
        }

        let winner = names.pop().unwrap_or_default();

        tree.conflicts.push(Conflict {
            path,
            winner,
            losers: names,
        });
    }

    Ok(tree)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    root: &Path,
    dir: &Path,
    depth: usize,
    tree: &mut MergeTree,
    owner: &DeployMod,
    providers: &mut BTreeMap<String, Vec<String>>,
    contributed: &mut usize,
) -> AppResult<()> {
    if depth > MAX_DEPTH {
        note_skip(tree, format!("{}: nested too deeply", owner.name));

        return Ok(());
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        // A mod whose staging folder has not been created yet is not an error;
        // it is a mod that has not been downloaded. The caller decides whether
        // that matters.
        return Ok(());
    };

    for entry in entries.flatten() {
        if tree.files.len() >= MAX_FILES {
            return Err(AppError::invalid(format!(
                "This sandbox would deploy more than {MAX_FILES} files, which is more than the \
                 app will manage at once."
            )));
        }

        let Ok(meta) = entry.metadata() else { continue };
        let path = entry.path();

        /*
         * SYMLINKS IN STAGING ARE SKIPPED, ALWAYS.
         *
         * `entry.metadata()` follows links, so this check uses the file TYPE
         * from `symlink_metadata` instead. A link here would be hard-linked or
         * copied into the game directory pointing at whatever it names — which
         * is a write outside every root this app validated, arriving through a
         * file the executor put in a jail it was allowed to write to. The jail
         * stops an installer WRITING outside; this is what stops it planting a
         * pointer for the deployment step to follow.
         *
         * Nothing legitimate needs one: staging holds a mod's files, and a mod
         * that ships a symlink to somewhere on the player's disk is not a mod.
         */
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            note_skip(
                tree,
                format!(
                    "{}: {} is a link and was skipped",
                    owner.name,
                    name_of(&path)
                ),
            );

            continue;
        }

        if meta.is_dir() {
            walk(root, &path, depth + 1, tree, owner, providers, contributed)?;

            continue;
        }

        if !meta.is_file() {
            note_skip(
                tree,
                format!("{}: {} is not a regular file", owner.name, name_of(&path)),
            );

            continue;
        }

        let Some(rel) = relative_key(root, &path) else {
            note_skip(
                tree,
                format!("{}: {} has an unusable name", owner.name, name_of(&path)),
            );

            continue;
        };

        providers
            .entry(rel.clone())
            .or_default()
            .push(owner.name.clone());

        tree.files.insert(
            rel,
            Winner {
                mod_key: owner.key.clone(),
                mod_name: owner.name.clone(),
                source: path,
                size: meta.len(),
            },
        );

        *contributed += 1;
    }

    Ok(())
}

fn note_skip(tree: &mut MergeTree, reason: String) {
    if tree.skipped.len() < MAX_SKIPPED {
        tree.skipped.push(reason);
    } else if tree.skipped.len() == MAX_SKIPPED {
        tree.skipped.push("… and more".into());
    }
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// `root`-relative, `/`-separated, and refused if any component is unusable.
///
/// The result is stored in a database and later joined back onto a game
/// directory, so it goes through the same component rules the plugin jail
/// applies — a name that Windows would silently rewrite (trailing dot, an
/// alternate data stream colon) must not become a ledger row that then fails
/// to match the file it was supposed to describe.
fn relative_key(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;

    let mut parts: Vec<String> = Vec::new();

    for component in rel.components() {
        let std::path::Component::Normal(part) = component else {
            return None;
        };

        let text = part.to_str()?;

        if text.is_empty()
            || text.contains('\0')
            || text.contains(':')
            || text.ends_with('.')
            || text.ends_with(' ')
        {
            return None;
        }

        parts.push(text.to_string());
    }

    if parts.is_empty() {
        return None;
    }

    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    fn a_mod(root: &Path, key: &str, priority: i64) -> DeployMod {
        DeployMod {
            key: key.into(),
            name: key.into(),
            root: root.join(key),
            priority,
        }
    }

    #[test]
    fn the_highest_priority_provider_wins_a_shared_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "low/textures/sky.dds", "low");
        write(root, "high/textures/sky.dds", "high");
        write(root, "high/meshes/rock.nif", "rock");

        let tree = build(&[a_mod(root, "high", 2), a_mod(root, "low", 1)]).expect("tree");

        assert_eq!(tree.file_count(), 2);
        assert_eq!(
            tree.files
                .get("textures/sky.dds")
                .map(|w| w.mod_key.as_str()),
            Some("high")
        );

        assert_eq!(tree.conflicts.len(), 1);

        let conflict = &tree.conflicts[0];

        assert_eq!(conflict.path, "textures/sky.dds");
        assert_eq!(conflict.winner, "high");
        assert_eq!(conflict.losers, vec!["low".to_string()]);
    }

    /// The regression this ordering exists to prevent: passing the list in
    /// database order must produce the same tree as passing it sorted, or
    /// reordering mods in the UI would appear to do nothing.
    #[test]
    fn input_order_does_not_decide_the_winner() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "low/a.txt", "low");
        write(root, "high/a.txt", "high");

        let ascending = build(&[a_mod(root, "low", 1), a_mod(root, "high", 9)]).expect("tree");
        let descending = build(&[a_mod(root, "high", 9), a_mod(root, "low", 1)]).expect("tree");

        for tree in [&ascending, &descending] {
            assert_eq!(
                tree.files.get("a.txt").map(|w| w.mod_key.as_str()),
                Some("high")
            );
        }
    }

    #[test]
    fn equal_priorities_break_by_key_so_the_answer_is_stable() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        write(root, "aaa/x.txt", "a");
        write(root, "zzz/x.txt", "z");

        let first = build(&[a_mod(root, "aaa", 5), a_mod(root, "zzz", 5)]).expect("tree");
        let second = build(&[a_mod(root, "zzz", 5), a_mod(root, "aaa", 5)]).expect("tree");

        // `zzz` sorts last, so it is applied last and wins — the same way both
        // times, which is the property that matters.
        assert_eq!(
            first.files.get("x.txt").map(|w| w.mod_key.as_str()),
            Some("zzz")
        );
        assert_eq!(
            second.files.get("x.txt").map(|w| w.mod_key.as_str()),
            Some("zzz")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_in_staging_is_never_deployed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let outside = tempfile::tempdir().expect("outside");

        let secret = outside.path().join("id_rsa");
        std::fs::write(&secret, b"private").expect("write");

        std::fs::create_dir_all(root.join("evil")).expect("mkdir");
        std::os::unix::fs::symlink(&secret, root.join("evil/key.txt")).expect("symlink");

        // And a directory link, which would otherwise pull in a whole tree.
        std::os::unix::fs::symlink(outside.path(), root.join("evil/elsewhere")).expect("symlink");

        let tree = build(&[a_mod(root, "evil", 1)]).expect("tree");

        assert_eq!(
            tree.file_count(),
            0,
            "a link must not become a deployed file"
        );
        assert_eq!(tree.empty, vec!["evil".to_string()]);
        assert!(!tree.skipped.is_empty(), "and it must be reported");
    }

    #[test]
    fn a_mod_with_no_files_is_reported_rather_than_ignored() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        std::fs::create_dir_all(root.join("empty")).expect("mkdir");
        write(root, "real/a.txt", "a");

        let tree = build(&[a_mod(root, "empty", 1), a_mod(root, "real", 2)]).expect("tree");

        assert_eq!(tree.empty, vec!["empty".to_string()]);
        assert_eq!(tree.file_count(), 1);
    }

    #[test]
    fn relative_keys_are_slash_separated_and_reject_odd_names() {
        let root = Path::new("/base");

        assert_eq!(
            relative_key(root, Path::new("/base/a/b/c.txt")).as_deref(),
            Some("a/b/c.txt")
        );

        // Nothing outside the root, and nothing Windows would rewrite.
        assert!(relative_key(root, Path::new("/elsewhere/x")).is_none());
        assert!(relative_key(root, Path::new("/base")).is_none());
    }
}
