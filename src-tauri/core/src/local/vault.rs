//! Paths this process found, addressable from the webview by token.
//!
//! THE PROBLEM THIS SOLVES
//! -----------------------
//! `commands/mod.rs`'s second rule is that **no command takes a filesystem path
//! from the webview**. Dropping a file into the window and importing a folder
//! another mod manager owns both look like exceptions to that rule, and neither
//! one has to be.
//!
//! In both cases the path was produced by *this side*: the OS delivered a drag
//! and drop event to the window, or this crate's own scan listed a directory.
//! The webview's job is only to say *which of the things you found* to act on.
//! So it gets a random token per path and hands that back.
//!
//! WHAT THAT DOES AND DOES NOT BUY
//! -------------------------------
//! It is worth being exact, because a token store is easy to over-claim.
//!
//! A script that has achieved execution in the webview — the threat model the
//! whole architecture assumes — can replay a token for a file **the user
//! themselves just dropped**. That is not a new capability: the same script
//! could ask them to drop it again. What it cannot do is name
//! `~/.ssh/id_ed25519`, or `../../etc/shadow`, or the settings file that
//! anchors the plugin jail, because **no token exists for any of them**. The
//! set of reachable paths is exactly the set a person pointed at, and the
//! webview never learns what any of them actually are.
//!
//! It is not an authorisation check and does not pretend to be one. Every
//! import still copies through [`super::store`], which bounds size, depth and
//! entry count and refuses a traversal.
//!
//! WHY TOKENS EXPIRE, AND WHY THE STORE IS BOUNDED
//! ----------------------------------------------
//! A token is a capability, and a capability with no lifetime is one an
//! injected script can hoard against a later moment. Tokens are dropped
//! oldest-first past a cap and refused past an age, so the window in which one
//! is useful is roughly the window in which somebody is looking at the dialog
//! it was minted for.
//!
//! WHY THEY ARE NOT CONSUMED ON READ
//! ---------------------------------
//! An import dialog reads a batch to show it, and then reads the chosen subset
//! again to act on it. Consuming on the first read would make the second fail,
//! which surfaces as "the import button works only sometimes" — a bug that is
//! very hard to attribute and buys nothing: a token that is still in the vault
//! names a path the user chose a moment ago either way.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How many paths the vault remembers.
///
/// A drop of a whole mods folder is a few hundred entries; past this the oldest
/// are forgotten, which costs a re-drop and nothing else.
pub const MAX_ENTRIES: usize = 4_000;

/// How long a token stays usable — long enough to read an import dialog and
/// think about it, far short of "for the life of the process".
pub const TTL: Duration = Duration::from_secs(30 * 60);

pub struct PathVault {
    entries: Mutex<VecDeque<Entry>>,
    ttl: Duration,
}

struct Entry {
    token: String,
    path: PathBuf,
    at: Instant,
}

impl PathVault {
    pub fn new() -> Self {
        Self::with_ttl(TTL)
    }

    /// A vault with a different lifetime. Exists so the expiry can be tested
    /// without a test that sleeps for half an hour.
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            ttl,
        }
    }

    /// Remember a path and return the token that names it.
    ///
    /// The token is 144 bits from the OS's generator. Not a counter and not a
    /// hash of the path: a guessable token is a path the webview can name
    /// without anybody having pointed at it, which is the one thing this exists
    /// to prevent.
    pub fn mint(&self, path: impl Into<PathBuf>) -> String {
        use base64::Engine as _;
        use rand::RngCore as _;

        let mut bytes = [0u8; 18];
        rand::rngs::OsRng.fill_bytes(&mut bytes);

        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);

        if let Ok(mut entries) = self.entries.lock() {
            entries.push_back(Entry {
                token: token.clone(),
                path: path.into(),
                at: Instant::now(),
            });

            while entries.len() > MAX_ENTRIES {
                entries.pop_front();
            }
        }

        token
    }

    /// Mint a token for each of several paths, in order.
    pub fn mint_all<P: Into<PathBuf>>(&self, paths: impl IntoIterator<Item = P>) -> Vec<String> {
        paths.into_iter().map(|p| self.mint(p)).collect()
    }

    /// The path a token names, if it still names one.
    pub fn resolve(&self, token: &str) -> Option<PathBuf> {
        let mut entries = self.entries.lock().ok()?;

        self.sweep(&mut entries);

        entries
            .iter()
            .find(|e| e.token == token)
            .map(|e| e.path.clone())
    }

    /// Resolve several, reporting the ones that no longer resolve.
    ///
    /// Both halves, because a partly-expired batch is a real case — somebody
    /// left the import dialog open — and silently importing the survivors is
    /// how a user ends up with eleven of the twelve mods they picked and no
    /// idea which one is missing.
    pub fn resolve_all(&self, tokens: &[String]) -> (Vec<PathBuf>, Vec<String>) {
        let mut found = Vec::new();
        let mut missing = Vec::new();

        for token in tokens {
            match self.resolve(token) {
                Some(path) => found.push(path),
                None => missing.push(token.clone()),
            }
        }

        (found, missing)
    }

    /// How many live tokens there are. For tests and for the log.
    pub fn len(&self) -> usize {
        let Ok(mut entries) = self.entries.lock() else {
            return 0;
        };

        self.sweep(&mut entries);

        entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Sweep on read rather than on a timer.
    ///
    /// These are the only functions that touch the vault, so there is nowhere
    /// else the expiry could happen without a thread whose whole job is to wake
    /// up and find nothing to do.
    fn sweep(&self, entries: &mut VecDeque<Entry>) {
        let now = Instant::now();
        let ttl = self.ttl;

        entries.retain(|e| now.duration_since(e.at) < ttl);
    }
}

impl Default for PathVault {
    fn default() -> Self {
        Self::new()
    }
}

/// Does `path` still exist and look like something worth importing?
///
/// Checked at mint time AND again at use time, because a drop event and an
/// import click are seconds apart and a temporary file can be gone in between.
pub fn is_importable(path: &Path) -> bool {
    // `symlink_metadata`, so a symlink is judged as a symlink rather than as
    // whatever it points at. Importing through one is how "this mod folder"
    // becomes "the filesystem".
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() || m.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_names_the_path_it_was_minted_for() {
        let vault = PathVault::new();

        let a = vault.mint("/tmp/one.zip");
        let b = vault.mint("/tmp/two.zip");

        assert_ne!(a, b, "tokens are not derived from the path");
        assert_eq!(vault.resolve(&a), Some(PathBuf::from("/tmp/one.zip")));
        assert_eq!(vault.resolve(&b), Some(PathBuf::from("/tmp/two.zip")));
    }

    /// The whole point: a path nobody pointed at has no token, so there is
    /// nothing the webview could send that would reach it.
    #[test]
    fn an_unminted_path_cannot_be_named() {
        let vault = PathVault::new();

        vault.mint("/tmp/one.zip");

        assert_eq!(vault.resolve("/etc/shadow"), None);
        assert_eq!(vault.resolve(""), None);
        assert_eq!(vault.resolve("AAAAAAAAAAAAAAAAAAAAAAAA"), None);
    }

    #[test]
    fn a_token_survives_being_read_twice() {
        let vault = PathVault::new();
        let token = vault.mint("/tmp/one.zip");

        assert!(vault.resolve(&token).is_some());
        assert!(
            vault.resolve(&token).is_some(),
            "the dialog reads a batch to show it and again to act on it"
        );
    }

    #[test]
    fn an_expired_token_stops_working() {
        let vault = PathVault::with_ttl(Duration::from_millis(0));
        let token = vault.mint("/tmp/one.zip");

        assert_eq!(vault.resolve(&token), None);
        assert!(vault.is_empty());
    }

    #[test]
    fn the_vault_is_bounded_and_forgets_the_oldest() {
        let vault = PathVault::new();

        let first = vault.mint("/tmp/first.zip");

        for i in 0..MAX_ENTRIES {
            vault.mint(format!("/tmp/{i}.zip"));
        }

        assert_eq!(vault.len(), MAX_ENTRIES);
        assert_eq!(vault.resolve(&first), None);
    }

    /// A partly-expired batch has to say which half went, not silently act on
    /// the survivors.
    #[test]
    fn resolving_a_batch_reports_what_it_could_not_find() {
        let vault = PathVault::new();

        let good = vault.mint("/tmp/one.zip");
        let tokens = vec![good.clone(), "not-a-token".to_string()];

        let (found, missing) = vault.resolve_all(&tokens);

        assert_eq!(found, vec![PathBuf::from("/tmp/one.zip")]);
        assert_eq!(missing, vec!["not-a-token".to_string()]);
    }
}
