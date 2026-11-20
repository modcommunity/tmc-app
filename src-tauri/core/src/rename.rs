//! The move from the app's old name to the GameSpice App's own directories.
//!
//! Builds before the rename kept everything under a directory named after the
//! bundle identifier (`com.moddingcommunity.app`), which Tauri's path resolver
//! derives on its own. The identifier itself stays — changing it would make the
//! OS treat the GameSpice App as a different program, and the updater would
//! install it beside the old one rather than over it — so the desktop build
//! names its directories itself, and moves an existing install's data across
//! the first time it starts.
//!
//! **The move is one-way and leaves a link behind.** The library database
//! records absolute paths into the data directory (installed games, backups),
//! and a symlink-strategy sandbox has links in the user's game folders pointing
//! at its staging tree. Renaming the directory alone would orphan all of them,
//! so after the rename the old path is made a symlink to the new one; anything
//! that still names the old path keeps resolving. Where the link cannot be made
//! (Windows without the privilege to create one), the rename is undone and the
//! app keeps using the old directory — a working install under the old name is
//! better than a renamed one with dangling paths.
//!
//! Mobile is out of scope: there the directories live inside the app's
//! container, which the unchanged identifier already names, and no user sees
//! them.

use std::path::{Component, Path, PathBuf};

/// The bundle identifier the old directories were named after.
pub const LEGACY_DIR: &str = "com.moddingcommunity.app";

/// What the GameSpice App's own directories are called.
pub const DIR: &str = "gamespice-app";

/// The keychain service the refresh token was stored under before the rename.
pub const LEGACY_KEYRING_SERVICE: &str = "com.moddingcommunity.app";

/// The keychain service the GameSpice App stores credentials under.
pub const KEYRING_SERVICE: &str = "gamespice-app";

/// `legacy` with its identifier component replaced by [`DIR`].
///
/// The identifier is not always the last component — on Linux the log
/// directory is `~/.local/share/<identifier>/logs` — so every component is
/// checked. `None` when the path does not contain the identifier at all, in
/// which case it is not one of ours to rename.
pub fn rebase(legacy: &Path) -> Option<PathBuf> {
    let mut found = false;
    let mut out = PathBuf::new();

    for component in legacy.components() {
        match component {
            Component::Normal(name) if name == LEGACY_DIR => {
                found = true;
                out.push(DIR);
            }
            other => out.push(other.as_os_str()),
        }
    }

    found.then_some(out)
}

/// What [`migrate_dir`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Migration {
    /// Nothing to move: no old directory, or the new one already exists.
    NotNeeded,
    /// Moved, and the old path now links to the new one.
    Moved,
    /// Could not be moved safely; keep using the old directory.
    KeptLegacy(String),
}

/// Move `old` to `new` and leave a link at `old`, once.
///
/// Idempotent: a second call finds `new` present (or `old` already a link) and
/// does nothing. `new` existing beside a real `old` means a fresh install ran
/// before an old one's data appeared, or the user put it back by hand; neither
/// is merged, because merging two libraries is not a thing a startup step can
/// do correctly.
pub fn migrate_dir(old: &Path, new: &Path) -> Migration {
    let Ok(meta) = std::fs::symlink_metadata(old) else {
        return Migration::NotNeeded;
    };

    if meta.file_type().is_symlink() || !meta.is_dir() || new.exists() {
        return Migration::NotNeeded;
    }

    if let Some(parent) = new.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Migration::KeptLegacy(format!("create {}: {e}", parent.display()));
        }
    }

    if let Err(e) = std::fs::rename(old, new) {
        // Another volume, or a file held open (Windows). Nothing moved.
        return Migration::KeptLegacy(format!("rename: {e}"));
    }

    match link_dir(new, old) {
        Ok(()) => Migration::Moved,
        Err(e) => {
            // Put it back, so nothing that names the old path breaks.
            match std::fs::rename(new, old) {
                Ok(()) => Migration::KeptLegacy(format!("link back: {e}")),
                // Moved but unlinked, and could not undo: the new path is the
                // only copy now, so it is the one to use.
                Err(undo) => {
                    tracing::error!(
                        "renamed data dir but could not link back ({e}) or undo ({undo})"
                    );
                    Migration::Moved
                }
            }
        }
    }
}

/// Move one file (the audit log) when its directory was not moved with the
/// data. A copy-then-delete fallback, because a log on another volume is the
/// ordinary case on Windows (`Local` vs `Roaming`) only in principle, but costs
/// nothing to handle.
pub fn migrate_file(old: &Path, new: &Path) -> bool {
    if new.exists() || !old.is_file() {
        return false;
    }

    if let Some(parent) = new.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }

    if std::fs::rename(old, new).is_ok() {
        return true;
    }

    std::fs::copy(old, new).is_ok() && std::fs::remove_file(old).is_ok()
}

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(not(any(unix, windows)))]
fn link_dir(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("no directory links on this platform"))
}

/// Read a credential from the new keychain service, falling back to the old one
/// once and moving it across.
///
/// Generic over the store so the order — new first, then old, then write new
/// and only then delete old — is tested without a keychain. A failed write
/// leaves the old entry in place, so a sign-in is never lost to a half move.
pub fn migrate_secret<G, S, D>(get: G, set_new: S, delete_old: D) -> Option<String>
where
    G: Fn(bool) -> Option<String>,
    S: FnOnce(&str) -> bool,
    D: FnOnce(),
{
    if let Some(value) = get(false) {
        return Some(value);
    }

    let value = get(true)?;

    if set_new(&value) {
        delete_old();
    }

    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebase_replaces_the_identifier_wherever_it_is() {
        assert_eq!(
            rebase(Path::new(
                "/home/u/.local/share/com.moddingcommunity.app/logs"
            )),
            Some(PathBuf::from("/home/u/.local/share/gamespice-app/logs"))
        );
        assert_eq!(
            rebase(Path::new("/home/u/.config/com.moddingcommunity.app")),
            Some(PathBuf::from("/home/u/.config/gamespice-app"))
        );
        assert_eq!(rebase(Path::new("/home/u/.config/other")), None);
    }

    #[test]
    fn an_old_install_is_moved_and_the_old_path_still_resolves() {
        let tmp = tempfile::tempdir().expect("tmp");
        let old = tmp.path().join(LEGACY_DIR);
        let new = tmp.path().join(DIR);

        std::fs::create_dir_all(old.join("games/x")).expect("mkdir");
        std::fs::write(old.join("library.sqlite3"), b"db").expect("write");
        std::fs::write(old.join("games/x/run.sh"), b"#!").expect("write");

        assert_eq!(migrate_dir(&old, &new), Migration::Moved);
        assert_eq!(
            std::fs::read(new.join("library.sqlite3")).expect("read"),
            b"db"
        );

        // An absolute path recorded before the move still opens.
        assert_eq!(
            std::fs::read(old.join("games/x/run.sh")).expect("read"),
            b"#!"
        );
        assert!(std::fs::symlink_metadata(&old)
            .expect("meta")
            .file_type()
            .is_symlink());

        // Second start: nothing to do, nothing disturbed.
        assert_eq!(migrate_dir(&old, &new), Migration::NotNeeded);
        assert!(new.join("library.sqlite3").is_file());
    }

    #[test]
    fn a_fresh_install_and_an_existing_new_dir_are_left_alone() {
        let tmp = tempfile::tempdir().expect("tmp");
        let old = tmp.path().join(LEGACY_DIR);
        let new = tmp.path().join(DIR);

        assert_eq!(migrate_dir(&old, &new), Migration::NotNeeded);

        std::fs::create_dir_all(&old).expect("mkdir");
        std::fs::create_dir_all(&new).expect("mkdir");
        std::fs::write(old.join("settings.json"), b"{}").expect("write");

        assert_eq!(migrate_dir(&old, &new), Migration::NotNeeded);
        assert!(
            old.join("settings.json").is_file(),
            "never merged or deleted"
        );
    }

    #[test]
    fn a_log_file_moves_once() {
        let tmp = tempfile::tempdir().expect("tmp");
        let old = tmp.path().join("a/activity.jsonl");
        let new = tmp.path().join("b/activity.jsonl");

        std::fs::create_dir_all(old.parent().expect("parent")).expect("mkdir");
        std::fs::write(&old, b"line\n").expect("write");

        assert!(migrate_file(&old, &new));
        assert!(!old.exists());
        assert!(!migrate_file(&old, &new));
        assert_eq!(std::fs::read(&new).expect("read"), b"line\n");
    }

    #[test]
    fn a_secret_moves_from_the_old_service_only_after_the_new_write() {
        use std::cell::RefCell;

        let new: RefCell<Option<String>> = RefCell::new(None);
        let old: RefCell<Option<String>> = RefCell::new(Some("tok".into()));

        let got = migrate_secret(
            |legacy| {
                if legacy {
                    old.borrow().clone()
                } else {
                    new.borrow().clone()
                }
            },
            |v| {
                *new.borrow_mut() = Some(v.to_string());
                true
            },
            || *old.borrow_mut() = None,
        );

        assert_eq!(got.as_deref(), Some("tok"));
        assert_eq!(new.borrow().as_deref(), Some("tok"));
        assert!(old.borrow().is_none());

        // A failed write keeps the old entry.
        let old2: RefCell<Option<String>> = RefCell::new(Some("tok".into()));
        let got = migrate_secret(
            |legacy| if legacy { old2.borrow().clone() } else { None },
            |_| false,
            || *old2.borrow_mut() = None,
        );
        assert_eq!(got.as_deref(), Some("tok"));
        assert!(old2.borrow().is_some());
    }
}
