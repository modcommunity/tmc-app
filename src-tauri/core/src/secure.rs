//! Where the refresh token lives.
//!
//! The access token is never persisted at all — it lasts an hour and is held in
//! memory by [`crate::auth::AuthState`]. Only the refresh half survives a
//! restart, and only in the strongest store the platform offers:
//!
//!   * **Desktop** — the OS credential store (Keychain / Credential Manager /
//!     Secret Service). Encrypted at rest by the OS, unlocked with the login
//!     session, and readable only by this app's identity.
//!   * **Mobile** — a file inside the app's private container. Android and iOS
//!     isolate that directory per-app at the kernel level and encrypt it with
//!     the device credential, which is the same guarantee the desktop keychain
//!     gives; a Keystore/Keychain round-trip on top of it would add ceremony
//!     without adding a boundary.
//!
//! Neither branch is reachable from the webview. There is no command that
//! returns a token to JavaScript — the HTTP client attaches it in Rust (see
//! [`crate::api`]), so an XSS in a rendered mod description has nothing to
//! steal.
//!
//! **The store is keyed on which site the build talks to.** A production token
//! and a dev-server token are different credentials for different systems, and
//! [`crate::api::api_base_scope`] keeps them in separate entries. Sharing one
//! entry meant a developer pointing a signed-in app at a dev instance sent the
//! REAL refresh token to it on the first refresh, and — since a rejected
//! refresh is terminal — got signed out of the real site for their trouble.

use std::path::PathBuf;

// `AppError` is referenced through its full path below rather than imported:
// the only use is inside an `os-keyring` block, and an import would be an
// unused-import warning everywhere that feature is off.
use crate::error::AppResult;

// Referenced only by the `os-keyring` code paths, which are compiled out on
// mobile and in `cargo test -p tmc-core`.
#[cfg(feature = "os-keyring")]
const SERVICE: &str = "com.moddingcommunity.app";
#[cfg(feature = "os-keyring")]
const ACCOUNT: &str = "refresh-token";

/// The keychain entry for THIS build's API base.
///
/// Production keeps the bare name so an existing install's session survives.
#[cfg(feature = "os-keyring")]
fn account() -> String {
    match crate::api::api_base_scope() {
        Some(scope) => format!("{ACCOUNT}@{scope}"),
        None => ACCOUNT.to_string(),
    }
}

/// A secret name that is safe as both a keychain account and a filename.
///
/// These names are ours, not a user's — but the fallback branch turns one into
/// a path, and a function that can only ever be called with a constant is
/// exactly the one that gets a variable passed to it later.
fn safe_name(name: &str) -> AppResult<String> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');

    if !ok {
        return Err(crate::error::AppError::invalid("Bad secret name."));
    }

    Ok(name.to_string())
}

/// A named secret's keychain account, split per API base like the token's is.
#[cfg(feature = "os-keyring")]
fn scoped(name: &str) -> String {
    match crate::api::api_base_scope() {
        Some(scope) => format!("{name}@{scope}"),
        None => name.to_string(),
    }
}

pub struct SecureStore {
    /// The whole store on mobile; the fallback on a desktop with no credential
    /// service running.
    fallback: PathBuf,
    /// Where a named secret's fallback file goes, when the keychain is not
    /// available. Its own directory so a listing of the data dir does not read
    /// as a list of what the app holds.
    secrets: PathBuf,
}

impl SecureStore {
    pub fn new(data_dir: &std::path::Path) -> Self {
        // Same split as the keychain entry, for the same reason: a dev build's
        // session must not overwrite — or be readable as — the real one.
        let file = match crate::api::api_base_scope() {
            Some(scope) => format!("credentials-{scope}.bin"),
            None => "credentials.bin".to_string(),
        };

        Self {
            fallback: data_dir.join(file),
            secrets: data_dir.join("secrets"),
        }
    }

    // ------------------------------------------------------- Named secrets
    //
    // The refresh token above has its own methods because it predates these and
    // because its keychain entry name is load-bearing for existing installs.
    // Everything else the app has to keep — currently the key that encrypts
    // RCON passwords — goes through the pair below.

    /// Store `value` under `name`, in the strongest place the platform offers.
    pub fn save_named(&self, name: &str, value: &str) -> AppResult<()> {
        let name = safe_name(name)?;

        #[cfg(all(
            feature = "os-keyring",
            any(target_os = "windows", target_os = "macos", target_os = "linux")
        ))]
        {
            if let Ok(entry) = keyring::Entry::new(SERVICE, &scoped(&name)) {
                if entry.set_password(value).is_ok() {
                    // Remove any fallback copy, so there is one answer rather
                    // than two that can disagree.
                    let _ = std::fs::remove_file(self.secrets.join(&name));

                    return Ok(());
                }
            }

            tracing::warn!("credential store unavailable for '{name}'; using data dir");
        }

        std::fs::create_dir_all(&self.secrets)?;

        let path = self.secrets.join(&name);

        std::fs::write(&path, value.as_bytes())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    pub fn load_named(&self, name: &str) -> Option<String> {
        let name = safe_name(name).ok()?;

        #[cfg(all(
            feature = "os-keyring",
            any(target_os = "windows", target_os = "macos", target_os = "linux")
        ))]
        {
            if let Ok(entry) = keyring::Entry::new(SERVICE, &scoped(&name)) {
                match entry.get_password() {
                    Ok(value) => return Some(value),
                    Err(keyring::Error::NoEntry) => {}
                    Err(e) => tracing::warn!("credential store read '{name}': {e}"),
                }
            }
        }

        std::fs::read_to_string(self.secrets.join(&name))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn clear_named(&self, name: &str) {
        let Ok(name) = safe_name(name) else { return };

        #[cfg(all(
            feature = "os-keyring",
            any(target_os = "windows", target_os = "macos", target_os = "linux")
        ))]
        {
            if let Ok(entry) = keyring::Entry::new(SERVICE, &scoped(&name)) {
                let _ = entry.delete_credential();
            }
        }

        let _ = std::fs::remove_file(self.secrets.join(&name));
    }

    pub fn save(&self, token: &str) -> AppResult<()> {
        #[cfg(all(
            feature = "os-keyring",
            any(target_os = "windows", target_os = "macos", target_os = "linux")
        ))]
        {
            match keyring::Entry::new(SERVICE, &account()) {
                Ok(entry) => {
                    return entry.set_password(token).map_err(|e| {
                        crate::error::AppError::internal(format!("keyring set: {e}"))
                    });
                }
                Err(e) => {
                    /*
                     * A Linux desktop with no Secret Service running (a bare
                     * WM, a container, some minimal distros) has no credential
                     * store at all. Refusing to sign in would be the wrong
                     * answer for a user who does not have one and cannot
                     * install one; falling back to the private data dir keeps
                     * the app usable at a stated, lower guarantee.
                     */
                    tracing::warn!("credential store unavailable ({e}); using data dir");
                }
            }
        }

        self.save_file(token)
    }

    pub fn load(&self) -> Option<String> {
        #[cfg(all(
            feature = "os-keyring",
            any(target_os = "windows", target_os = "macos", target_os = "linux")
        ))]
        {
            if let Ok(entry) = keyring::Entry::new(SERVICE, &account()) {
                match entry.get_password() {
                    Ok(token) => return Some(token),
                    // `NoEntry` is the ordinary "never signed in" case.
                    Err(keyring::Error::NoEntry) => {}
                    Err(e) => tracing::warn!("credential store read: {e}"),
                }
            }
        }

        self.load_file()
    }

    pub fn clear(&self) {
        #[cfg(all(
            feature = "os-keyring",
            any(target_os = "windows", target_os = "macos", target_os = "linux")
        ))]
        {
            if let Ok(entry) = keyring::Entry::new(SERVICE, &account()) {
                let _ = entry.delete_credential();
            }
        }

        let _ = std::fs::remove_file(&self.fallback);
    }

    fn save_file(&self, token: &str) -> AppResult<()> {
        if let Some(parent) = self.fallback.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&self.fallback, token.as_bytes())?;

        // 0600. The directory is already private on every supported platform,
        // but a shared-home Linux box is exactly where that assumption breaks.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let _ =
                std::fs::set_permissions(&self.fallback, std::fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    fn load_file(&self) -> Option<String> {
        std::fs::read_to_string(&self.fallback)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}
