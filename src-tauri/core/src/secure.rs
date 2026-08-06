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

pub struct SecureStore {
    /// The whole store on mobile; the fallback on a desktop with no credential
    /// service running.
    fallback: PathBuf,
}

impl SecureStore {
    pub fn new(data_dir: &std::path::Path) -> Self {
        Self {
            fallback: data_dir.join("credentials.bin"),
        }
    }

    pub fn save(&self, token: &str) -> AppResult<()> {
        #[cfg(all(
            feature = "os-keyring",
            any(target_os = "windows", target_os = "macos", target_os = "linux")
        ))]
        {
            match keyring::Entry::new(SERVICE, ACCOUNT) {
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
            if let Ok(entry) = keyring::Entry::new(SERVICE, ACCOUNT) {
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
            if let Ok(entry) = keyring::Entry::new(SERVICE, ACCOUNT) {
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
