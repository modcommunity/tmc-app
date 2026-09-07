//! What a jail root is allowed to be.
//!
//! A game directory and the download directory are not ordinary preferences.
//! They are the **anchors of the plugin jail**: `plugins::jail` resolves
//! every `PathRef { root: gameDir, .. }` beneath whatever is stored here, and an
//! installer plugin holding `{gameDir, "", write}` can write anywhere under it.
//! So the value decides how much a plugin's permissions are actually worth.
//!
//! WHAT THIS CAN AND CANNOT DEFEND
//! ------------------------------
//! It cannot establish that a human chose the path. That property came from the
//! OS folder dialog — the returned path was one a person physically clicked,
//! out of reach of anything running in the webview — and it is gone the moment
//! the picker is drawn by the app itself. Nothing in this module brings it back,
//! and it would be dishonest to imply otherwise.
//!
//! What it does instead is make the anchor's *identity* the thing that has to be
//! safe, rather than its provenance. A script that can name any path it likes
//! still cannot name one that encloses the app's own credentials, its plugin
//! registry, the user's whole home directory or a filesystem root — which is the
//! set of anchors that turn one over-broad grant into total compromise. A game
//! folder is a specific place well down the tree, and every rule below is a
//! restatement of that.
//!
//! It lives in `tmc-core`, with no Tauri dependency, so the policy compiles and
//! is tested on a CI runner with no display stack.

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

/// Directories that must never *be* a root, though their children may be.
///
/// Exact matches only. Games legitimately live in `C:\Program Files\...` and
/// `/opt/...`; what must not happen is the anchor sitting AT one of these, where
/// the jail would span every unrelated program on the machine.
#[cfg(not(windows))]
const SYSTEM_DIRS: &[&str] = &[
    "/",
    "/bin",
    "/boot",
    "/dev",
    "/etc",
    "/home",
    "/lib",
    "/lib32",
    "/lib64",
    "/media",
    "/mnt",
    "/opt",
    "/proc",
    "/root",
    "/run",
    "/sbin",
    "/srv",
    "/sys",
    "/tmp",
    "/usr",
    "/usr/bin",
    "/usr/lib",
    "/usr/local",
    "/usr/share",
    "/var",
    "/Applications",
    "/Library",
    "/System",
    "/Users",
    "/Volumes",
];

#[cfg(windows)]
const SYSTEM_DIRS: &[&str] = &[
    r"C:\",
    r"C:\Windows",
    r"C:\Windows\System32",
    r"C:\Program Files",
    r"C:\Program Files (x86)",
    r"C:\ProgramData",
    r"C:\Users",
];

/// Check a candidate jail root and return it canonicalised.
///
/// `protected` is every directory the app itself owns — its data, logs, cache
/// and plugin folders, plus the user's home. The candidate is refused when it
/// is one of them or an ANCESTOR of one, because a jail anchored above the
/// app's own files contains `settings.json`, the plugin registry and (on
/// mobile, where there is no OS keychain) the refresh token. A plugin able to
/// rewrite the registry is a plugin able to grant itself permissions.
///
/// The canonical path is what gets stored. Resolving here means the value in
/// `settings.json` is the same one `plugins::jail` will resolve to later —
/// a stored path full of `..` and symlinks could pass this check and then
/// canonicalise somewhere else when the jail is built.
pub fn validate_root(candidate: &str, protected: &[PathBuf]) -> AppResult<PathBuf> {
    let trimmed = candidate.trim();

    if trimmed.is_empty() {
        return Err(AppError::invalid("No folder was given."));
    }

    let dir = Path::new(trimmed)
        .canonicalize()
        .map_err(|_| AppError::invalid("That folder does not exist."))?;

    if !dir.is_dir() {
        return Err(AppError::invalid("That is not a folder."));
    }

    /*
     * A filesystem root has no parent. Checked separately from SYSTEM_DIRS
     * because the list cannot name every root: a Windows install may have any
     * drive letter, and a Unix one may have a candidate under a mount point
     * that is itself a root.
     */
    if dir.parent().is_none() {
        return Err(AppError::jail(
            "A whole drive cannot be used as a game folder. Choose the game's own directory."
                .to_string(),
        ));
    }

    for system in SYSTEM_DIRS {
        // Compared after canonicalising the system path too, so `/tmp` matching
        // a machine where it is a symlink to `/private/tmp` still holds.
        let Ok(resolved) = Path::new(system).canonicalize() else {
            continue;
        };

        if dir == resolved {
            return Err(AppError::jail(format!(
                "{} is a system folder. Choose the game's own directory inside it.",
                dir.display()
            )));
        }
    }

    for owned in protected {
        let Ok(resolved) = owned.canonicalize() else {
            continue;
        };

        /*
         * `resolved.starts_with(&dir)` — the direction matters and is easy to
         * write backwards. The question is "does the candidate CONTAIN
         * something the app owns", not "is the candidate inside one", so the
         * owned path is the one doing the starts_with. Equality is covered by
         * the same test.
         */
        if resolved.starts_with(&dir) {
            return Err(AppError::jail(format!(
                "{} contains the app's own files. Choose the game's own directory.",
                dir.display()
            )));
        }
    }

    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp directory that cleans itself up, so these tests need no dev-dep.
    struct TempTree(PathBuf);

    impl TempTree {
        fn new(tag: &str) -> Self {
            let base =
                std::env::temp_dir().join(format!("tmc-anchor-{tag}-{}", std::process::id()));

            std::fs::create_dir_all(&base).expect("temp dir");

            Self(base)
        }

        fn child(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);

            std::fs::create_dir_all(&path).expect("child dir");

            path
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_ordinary_game_folder_is_accepted_and_canonicalised() {
        let tree = TempTree::new("ok");
        let games = tree.child("Games/Rust");

        let out = validate_root(games.to_str().expect("utf8"), &[]).expect("accepted");

        assert_eq!(out, games.canonicalize().expect("canonical"));
    }

    #[test]
    fn a_folder_that_does_not_exist_is_refused() {
        let tree = TempTree::new("missing");
        let ghost = tree.0.join("not-here");

        assert!(validate_root(ghost.to_str().expect("utf8"), &[]).is_err());
    }

    #[test]
    fn an_empty_path_is_refused() {
        assert!(validate_root("   ", &[]).is_err());
    }

    #[test]
    fn a_filesystem_root_is_refused() {
        // `/` on Unix, and the current drive's root on Windows.
        let root = Path::new(".")
            .canonicalize()
            .expect("cwd")
            .ancestors()
            .last()
            .expect("root")
            .to_path_buf();

        assert!(validate_root(root.to_str().expect("utf8"), &[]).is_err());
    }

    /// The rule the whole module exists for: an anchor ABOVE the app's own
    /// files would put `settings.json`, the plugin registry and the mobile
    /// token fallback inside the plugin jail.
    #[test]
    fn a_folder_containing_the_apps_own_data_is_refused() {
        let tree = TempTree::new("encloses");
        let home = tree.child("home");
        let data = tree.child("home/.local/share/tmc");

        let protected = std::slice::from_ref(&data);

        assert!(validate_root(home.to_str().expect("utf8"), protected).is_err());

        // The app's own data directory itself, not merely an ancestor of it.
        assert!(validate_root(data.to_str().expect("utf8"), protected).is_err());
    }

    /// The inverse must still pass, or every game folder under a home directory
    /// would be refused — which is where games actually live.
    #[test]
    fn a_sibling_of_the_apps_data_is_accepted() {
        let tree = TempTree::new("sibling");
        let data = tree.child("home/.local/share/tmc");
        let games = tree.child("home/Games/Rust");

        assert!(validate_root(games.to_str().expect("utf8"), &[data]).is_ok());
    }

    #[test]
    fn traversal_cannot_smuggle_an_ancestor_past_the_check() {
        let tree = TempTree::new("traversal");
        let data = tree.child("home/.local/share/tmc");
        let games = tree.child("home/Games");

        // Resolves to `home`, which encloses the data directory — the check
        // runs on the canonical path, not the string it arrived as.
        let sneaky = games.join("..");

        assert!(validate_root(sneaky.to_str().expect("utf8"), &[data]).is_err());
    }
}
