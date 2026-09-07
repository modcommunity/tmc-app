//! The installation ledger: every file a deploy put into the game directory,
//! and what was there before it.
//!
//! WHY A LEDGER AND NOT A RESCAN
//! -----------------------------
//! Undeploying by rescanning the game directory cannot work, and it fails in
//! the direction that loses data. A rescan can see that `Data/textures/sky.dds`
//! exists; it cannot see whether the app put it there or whether it shipped
//! with the game, and guessing wrong either strands a modded file forever or
//! deletes a base-game asset. The ledger is the only thing that knows.
//!
//! WHAT MAKES A PURGE SAFE
//! -----------------------
//! Every removal is conditional on the file still being the one that was
//! deployed. Three checks, one per mechanism:
//!
//!   * a **symbolic link** must still point at the staging file it was created
//!     for;
//!   * a **hard link** must still share identity with its staging file;
//!   * a **copy** must still have the size and modification time it had when
//!     it was written.
//!
//! Anything else is left alone and reported. That covers the case that actually
//! happens — a user edited a deployed config by hand, or the game rewrote it on
//! first run — and it means a purge can never be the reason somebody loses
//! work. The cost is that the game folder is not always returned to a
//! byte-perfect stock state, which is the right trade: a stranded file is
//! visible and fixable, a deleted one is not.
//!
//! BACKUPS
//! -------
//! When a deploy has to displace a file that was already there and was not
//! ours, the original is MOVED into the backup store rather than overwritten,
//! and the ledger row names it. A purge puts it back. That is what makes the
//! direct strategy — writing into the real game folder — survivable at all.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::plugins::jail::join_relative;

use super::link::{self, LinkKind};

/// One deployed file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
    /// Relative to the game directory, `/`-separated.
    pub path: String,
    /// `hard`, `symbolic` or `copy`.
    pub kind: String,
    /// Which mod provided it, so a per-mod purge is possible.
    pub mod_key: String,
    /// Absolute path in the staging folder this came from.
    pub source: String,
    pub size: u64,
    /// Modification time of the DEPLOYED file, milliseconds since the epoch.
    /// The copy strategy's tamper check; unused by the link strategies, which
    /// have a stronger one.
    pub mtime_ms: i64,
    /// Absolute path of the file that was displaced to make room, if any.
    #[serde(default)]
    pub backup: Option<String>,
}

impl LedgerEntry {
    pub fn link_kind(&self) -> LinkKind {
        match self.kind.as_str() {
            "hard" => LinkKind::Hard,
            "symbolic" => LinkKind::Symbolic,
            _ => LinkKind::Copy,
        }
    }
}

/// What a purge did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeReport {
    pub removed: usize,
    pub restored: usize,
    /// Files left in place because they are no longer the ones we deployed.
    pub kept: Vec<String>,
    /// Files the ledger named that were already gone. Not an error — a user
    /// deleting a mod file by hand is ordinary — but worth reporting, because
    /// a lot of them at once means something else is managing the same folder.
    pub missing: usize,
    pub errors: Vec<String>,
}

/// Millisecond mtime of a path, or 0.
///
/// Zero rather than an error: an unreadable timestamp must not fail a deploy,
/// and a recorded 0 simply means the copy strategy's tamper check will treat
/// the file as modified and leave it — the safe direction.
pub fn mtime_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Is the file at `abs` still the one this entry describes?
pub fn is_still_ours(entry: &LedgerEntry, abs: &Path) -> bool {
    match entry.link_kind() {
        LinkKind::Symbolic => {
            link::link_target(abs).is_some_and(|target| target == Path::new(&entry.source))
        }

        LinkKind::Hard => {
            let source = Path::new(&entry.source);

            /*
             * If the staging file is gone there is nothing to compare against,
             * and the honest answer is the recorded metadata — which is the
             * same test the copy strategy uses. A hard link whose source was
             * deleted is still a perfectly good file; the link is not broken,
             * it just no longer has a second name.
             */
            if source.exists() {
                link::same_file(abs, source)
            } else {
                matches_recorded(entry, abs)
            }
        }

        LinkKind::Copy => matches_recorded(entry, abs),
    }
}

fn matches_recorded(entry: &LedgerEntry, abs: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(abs) else {
        return false;
    };

    // A recorded mtime of 0 means "we could not read it", which must not match
    // everything — so it matches nothing.
    entry.mtime_ms != 0 && meta.len() == entry.size && mtime_ms(abs) == entry.mtime_ms
}

/// Remove everything these entries deployed, and put back what they displaced.
///
/// `target` is the game directory. Entries are processed deepest-first so a
/// directory is only considered for removal after its children are gone.
pub fn purge(target: &Path, entries: &[LedgerEntry]) -> PurgeReport {
    let mut report = PurgeReport::default();

    let mut ordered: Vec<&LedgerEntry> = entries.iter().collect();

    // Deepest first. `/` counts as one level, so a plain component count is
    // exactly the depth.
    ordered.sort_by_key(|e| std::cmp::Reverse(e.path.matches('/').count()));

    let mut parents: Vec<PathBuf> = Vec::new();

    for entry in ordered {
        /*
         * The ledger lives in a user-writable database, so its paths are
         * re-derived rather than trusted. `join_relative` is the same function
         * the plugin jail uses, and it refuses `..`, absolutes and every shape
         * Windows would rewrite — without it an edited row turns a purge into
         * a delete-anything primitive.
         */
        let Ok(abs) = join_relative(target, &entry.path) else {
            report.errors.push(format!(
                "{} is not a path inside the game folder.",
                entry.path
            ));

            continue;
        };

        if !abs.starts_with(target) {
            report
                .errors
                .push(format!("{} escapes the game folder.", entry.path));

            continue;
        }

        if let Some(parent) = abs.parent() {
            parents.push(parent.to_path_buf());
        }

        let exists = std::fs::symlink_metadata(&abs).is_ok();

        if !exists {
            report.missing += 1;
        } else if is_still_ours(entry, &abs) {
            match std::fs::remove_file(&abs) {
                Ok(()) => report.removed += 1,
                Err(e) => report.errors.push(format!("{}: {e}", entry.path)),
            }
        } else {
            report.kept.push(entry.path.clone());

            // A file that is no longer ours also owns its place: putting the
            // backup back would destroy whatever replaced it.
            continue;
        }

        if let Some(backup) = &entry.backup {
            restore(Path::new(backup), &abs, &mut report);
        }
    }

    prune_empty_dirs(target, &mut parents);

    report
}

fn restore(backup: &Path, abs: &Path, report: &mut PurgeReport) {
    if !backup.exists() {
        return;
    }

    if abs.exists() {
        // Something is in the way. Leave the backup where it is rather than
        // overwrite — it is the only copy of the original.
        report.errors.push(format!(
            "Could not restore {}: something else is in its place. The original is still at {}.",
            abs.display(),
            backup.display()
        ));

        return;
    }

    if let Some(parent) = abs.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match std::fs::rename(backup, abs) {
        Ok(()) => report.restored += 1,
        Err(_) => {
            // Across volumes `rename` fails; a copy-then-delete is the same
            // outcome and the backup store may well be on the app's drive.
            match std::fs::copy(backup, abs) {
                Ok(_) => {
                    let _ = std::fs::remove_file(backup);
                    report.restored += 1;
                }
                Err(e) => report
                    .errors
                    .push(format!("Could not restore {}: {e}", abs.display())),
            }
        }
    }
}

/// Remove directories the deploy created, deepest first, stopping at anything
/// that still holds a file.
///
/// `remove_dir` refuses a non-empty directory, which is the entire safety
/// mechanism here — a folder that still has the user's own files in it cannot
/// be removed by accident, so no bookkeeping about which directories we created
/// is needed.
fn prune_empty_dirs(target: &Path, parents: &mut Vec<PathBuf>) {
    parents.sort();
    parents.dedup();
    parents.sort_by_key(|p| std::cmp::Reverse(p.components().count()));

    for dir in parents.iter() {
        let mut cursor = dir.clone();

        while cursor.starts_with(target) && cursor != target {
            if std::fs::remove_dir(&cursor).is_err() {
                break;
            }

            match cursor.parent() {
                Some(parent) => cursor = parent.to_path_buf(),
                None => break,
            }
        }
    }
}

/// Move a file that is in the way into the backup store, returning where it
/// went.
///
/// A MOVE, never a copy: the point is that the original is no longer at the
/// path the deploy is about to write, and a copy would leave the deploy to
/// overwrite it — which is the exact data loss this exists to prevent.
pub fn back_up(abs: &Path, rel: &str, backup_root: &Path) -> AppResult<String> {
    let dest = join_relative(backup_root, rel)?;

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    /*
     * A backup already at that path is from an earlier deploy of the same
     * sandbox, and it is the OLDER one — which means it is the one closer to
     * the game's stock state, so it is the one worth keeping. Leaving it and
     * discarding the current file would be wrong too, so the current file is
     * parked beside it with a suffix and both are named in the ledger.
     */
    let dest = if dest.exists() {
        unique_beside(&dest)
    } else {
        dest
    };

    if std::fs::rename(abs, &dest).is_err() {
        std::fs::copy(abs, &dest)?;
        std::fs::remove_file(abs)?;
    }

    Ok(dest.to_string_lossy().into_owned())
}

fn unique_beside(path: &Path) -> PathBuf {
    for n in 1..1000u32 {
        let candidate = path.with_file_name(format!(
            "{}.{n}",
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "backup".into())
        ));

        if !candidate.exists() {
            return candidate;
        }
    }

    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: &str, source: &Path, abs: &Path) -> LedgerEntry {
        LedgerEntry {
            path: path.into(),
            kind: kind.into(),
            mod_key: "mod:1".into(),
            source: source.to_string_lossy().into_owned(),
            size: std::fs::metadata(abs).map(|m| m.len()).unwrap_or(0),
            mtime_ms: mtime_ms(abs),
            backup: None,
        }
    }

    #[test]
    fn a_copy_that_has_not_changed_is_removed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        let staging = tmp.path().join("staging");

        std::fs::create_dir_all(game.join("mods")).expect("mkdir");
        std::fs::create_dir_all(&staging).expect("mkdir");

        let source = staging.join("a.jar");
        std::fs::write(&source, b"jar").expect("write");

        let deployed = game.join("mods/a.jar");
        std::fs::copy(&source, &deployed).expect("copy");

        let ledger = vec![entry("mods/a.jar", "copy", &source, &deployed)];

        let report = purge(&game, &ledger);

        assert_eq!(report.removed, 1);
        assert!(!deployed.exists());
        // The now-empty directory goes too.
        assert!(!game.join("mods").exists());
    }

    #[test]
    fn a_copy_the_user_edited_is_left_alone() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        let staging = tmp.path().join("staging");

        std::fs::create_dir_all(&game).expect("mkdir");
        std::fs::create_dir_all(&staging).expect("mkdir");

        let source = staging.join("config.json");
        std::fs::write(&source, b"{}").expect("write");

        let deployed = game.join("config.json");
        std::fs::copy(&source, &deployed).expect("copy");

        let ledger = vec![entry("config.json", "copy", &source, &deployed)];

        // The user (or the game) rewrites it.
        std::fs::write(&deployed, b"{\"mine\":true}").expect("edit");

        let report = purge(&game, &ledger);

        assert_eq!(report.removed, 0);
        assert_eq!(report.kept, vec!["config.json".to_string()]);
        assert!(deployed.exists(), "an edited file must survive a purge");
    }

    #[test]
    fn a_displaced_original_comes_back() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        let staging = tmp.path().join("staging");
        let backups = tmp.path().join("backups");

        std::fs::create_dir_all(game.join("data")).expect("mkdir");
        std::fs::create_dir_all(&staging).expect("mkdir");

        let stock = game.join("data/base.pak");
        std::fs::write(&stock, b"stock").expect("write");

        let backup = back_up(&stock, "data/base.pak", &backups).expect("backed up");

        assert!(!stock.exists(), "the original is moved, not copied");

        let source = staging.join("base.pak");
        std::fs::write(&source, b"modded").expect("write");
        std::fs::copy(&source, &stock).expect("deploy");

        let mut row = entry("data/base.pak", "copy", &source, &stock);
        row.backup = Some(backup);

        let report = purge(&game, &[row]);

        assert_eq!(report.removed, 1);
        assert_eq!(report.restored, 1);
        assert_eq!(
            std::fs::read_to_string(&stock).expect("read"),
            "stock",
            "the game's own file must be back"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_somewhere_else_now_is_left_alone() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        let staging = tmp.path().join("staging");

        std::fs::create_dir_all(&game).expect("mkdir");
        std::fs::create_dir_all(&staging).expect("mkdir");

        let source = staging.join("a.jar");
        std::fs::write(&source, b"jar").expect("write");

        let deployed = game.join("a.jar");
        std::os::unix::fs::symlink(&source, &deployed).expect("symlink");

        let row = entry("a.jar", "symbolic", &source, &deployed);

        // Somebody repoints it.
        let other = staging.join("other.jar");
        std::fs::write(&other, b"other").expect("write");
        std::fs::remove_file(&deployed).expect("rm");
        std::os::unix::fs::symlink(&other, &deployed).expect("symlink");

        let report = purge(&game, &[row]);

        assert_eq!(report.removed, 0);
        assert_eq!(report.kept, vec!["a.jar".to_string()]);
    }

    /// The ledger is a file on the user's disk. An edited row must not be able
    /// to reach outside the game folder.
    #[test]
    fn a_tampered_ledger_row_cannot_delete_anything_outside_the_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        let outside = tmp.path().join("outside");

        std::fs::create_dir_all(&game).expect("mkdir");
        std::fs::create_dir_all(&outside).expect("mkdir");

        let victim = outside.join("important.txt");
        std::fs::write(&victim, b"keep me").expect("write");

        let rows: Vec<LedgerEntry> = ["../outside/important.txt", "/etc/passwd", "..\\..\\x"]
            .iter()
            .map(|p| LedgerEntry {
                path: (*p).into(),
                kind: "copy".into(),
                mod_key: "m".into(),
                source: String::new(),
                size: 7,
                mtime_ms: 1,
                backup: None,
            })
            .collect();

        let report = purge(&game, &rows);

        assert_eq!(report.removed, 0);
        assert_eq!(report.errors.len(), rows.len());
        assert!(victim.exists(), "a path outside the target must be refused");
    }

    #[test]
    fn a_file_already_gone_is_counted_not_an_error() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");

        std::fs::create_dir_all(&game).expect("mkdir");

        let row = LedgerEntry {
            path: "gone.jar".into(),
            kind: "copy".into(),
            mod_key: "m".into(),
            source: String::new(),
            size: 1,
            mtime_ms: 1,
            backup: None,
        };

        let report = purge(&game, &[row]);

        assert_eq!(report.missing, 1);
        assert!(report.errors.is_empty());
    }
}
