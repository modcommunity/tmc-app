//! The app's whole authentication state, and the PKCE machinery behind it.
//!
//! The design in one line: **no credential ever crosses the IPC boundary**.
//! The verifier is generated here, the tokens are stored here, and the bearer
//! header is attached here. The webview can start a login, ask whether one is
//! in progress, and read back a user profile — it cannot obtain a token, which
//! means an injected script in a rendered mod description has nothing to take.

use std::sync::RwLock;
use std::time::{Duration, Instant};

use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppResult;
use crate::secure::SecureStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUser {
    pub id: String,
    pub name: Option<String>,
    pub username: Option<String>,
    pub avatar: Option<String>,
    pub role: Option<String>,
}

/// What the app shows the user while the browser half is happening.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingLogin {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// The private half, never serialised.
struct PendingSecret {
    device_code: String,
    verifier: String,
    started: Instant,
    expires_in: Duration,
}

#[derive(Default)]
pub struct AuthState {
    /// Held in memory only. An hour-long credential is not worth persisting,
    /// and not persisting it means a stolen disk yields nothing usable without
    /// also defeating the credential store.
    access: RwLock<Option<AccessToken>>,
    refresh: RwLock<Option<String>>,
    user: RwLock<Option<SessionUser>>,
    pending: RwLock<Option<PendingSecret>>,
}

#[derive(Clone)]
struct AccessToken {
    value: String,
    /// When to stop using it. Deliberately pessimistic — see `is_fresh`.
    expires_at: Instant,
}

impl AuthState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restore the refresh token from the OS store. Does not contact the
    /// network; the first API call performs the exchange.
    pub fn restore(&self, store: &SecureStore) {
        if let Some(token) = store.load() {
            if let Ok(mut slot) = self.refresh.write() {
                *slot = Some(token);
            }
        }
    }

    pub fn has_refresh(&self) -> bool {
        self.refresh.read().is_ok_and(|r| r.is_some())
    }

    pub fn refresh_token(&self) -> Option<String> {
        self.refresh.read().ok().and_then(|r| r.clone())
    }

    pub fn user(&self) -> Option<SessionUser> {
        self.user.read().ok().and_then(|u| u.clone())
    }

    pub fn set_user(&self, user: Option<SessionUser>) {
        if let Ok(mut slot) = self.user.write() {
            *slot = user;
        }
    }

    /// The bearer to send, or `None` when one has to be minted first.
    ///
    /// Treated as stale 60s before it actually expires. The alternative is a
    /// request that leaves the app valid and arrives expired, which turns every
    /// slow connection into a spurious re-auth.
    pub fn access_token(&self) -> Option<String> {
        let guard = self.access.read().ok()?;
        let token = guard.as_ref()?;

        if token.expires_at.saturating_duration_since(Instant::now()) < Duration::from_secs(60) {
            return None;
        }

        Some(token.value.clone())
    }

    pub fn store_tokens(
        &self,
        store: &SecureStore,
        access: String,
        refresh: String,
        expires_in: u64,
    ) -> AppResult<()> {
        if let Ok(mut slot) = self.access.write() {
            *slot = Some(AccessToken {
                value: access,
                expires_at: Instant::now() + Duration::from_secs(expires_in),
            });
        }

        if let Ok(mut slot) = self.refresh.write() {
            *slot = Some(refresh.clone());
        }

        store.save(&refresh)
    }

    pub fn sign_out(&self, store: &SecureStore) {
        if let Ok(mut slot) = self.access.write() {
            *slot = None;
        }
        if let Ok(mut slot) = self.refresh.write() {
            *slot = None;
        }
        if let Ok(mut slot) = self.user.write() {
            *slot = None;
        }
        if let Ok(mut slot) = self.pending.write() {
            *slot = None;
        }

        store.clear();
    }

    // ------------------------------------------------------------- Device flow

    /// Generate a PKCE pair and remember the private half.
    ///
    /// Returns the challenge for the start request. Starting a second login
    /// discards the first — a user who abandoned a browser tab and pressed
    /// "Sign in" again must not leave a redeemable grant behind.
    pub fn begin(&self) -> AppResult<(String, String)> {
        let verifier = random_urlsafe(64);
        let challenge = pkce_challenge(&verifier);

        if let Ok(mut slot) = self.pending.write() {
            *slot = None;
        }

        Ok((verifier, challenge))
    }

    pub fn arm(&self, device_code: String, verifier: String, expires_in: u64) {
        if let Ok(mut slot) = self.pending.write() {
            *slot = Some(PendingSecret {
                device_code,
                verifier,
                started: Instant::now(),
                expires_in: Duration::from_secs(expires_in),
            });
        }
    }

    /// The `(device_code, verifier)` for the next poll, or `None` when no login
    /// is in flight or the grant has aged out locally.
    pub fn poll_pair(&self) -> Option<(String, String)> {
        let guard = self.pending.read().ok()?;
        let pending = guard.as_ref()?;

        if pending.started.elapsed() > pending.expires_in {
            return None;
        }

        Some((pending.device_code.clone(), pending.verifier.clone()))
    }

    pub fn clear_pending(&self) {
        if let Ok(mut slot) = self.pending.write() {
            *slot = None;
        }
    }

    pub fn is_pending(&self) -> bool {
        self.poll_pair().is_some()
    }
}

/// A base64url string with `bytes` bytes of entropy behind it.
fn random_urlsafe(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);

    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// S256, matching the server's `VerifyPkce`. Never `plain`: a leaked challenge
/// would then be the verifier, which defeats the point of having one.
fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());

    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Self-reported client identity, sent with the device-grant start.
///
/// Shown on the browser's approval screen. The server treats it as an
/// untrusted label, which is exactly right — it is a name for a human to
/// recognise, not an authorisation input.
pub fn client_info(version: &str) -> serde_json::Value {
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else {
        "unknown"
    };

    let name = hostname_label().unwrap_or_else(|| format!("TMC on {platform}"));

    serde_json::json!({
        "name": name,
        "platform": platform,
        "version": version,
    })
}

/// A recognisable device name, without pulling in a hostname crate.
///
/// Every branch is a best-effort read of something the platform already
/// exposes; a miss just means the approval screen says "TMC on linux", which is
/// still enough for the common case of approving the machine you are sitting at.
fn hostname_label() -> Option<String> {
    for key in ["COMPUTERNAME", "HOSTNAME", "HOST"] {
        if let Ok(v) = std::env::var(key) {
            if !v.trim().is_empty() {
                return Some(sanitise_label(&v));
            }
        }
    }

    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| sanitise_label(s.trim()))
        .filter(|s| !s.is_empty())
}

/// The server bounds this at 64 chars; trimming here means a long hostname
/// produces a readable name rather than a validation error the user cannot act
/// on. Control characters go because this string is rendered in a browser.
fn sanitise_label(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(48)
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_s256_base64url() {
        // Test vector from RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_is_within_the_servers_bounds() {
        let v = random_urlsafe(64);
        assert!((43..=128).contains(&v.len()), "len {}", v.len());
    }

    #[test]
    fn labels_are_bounded_and_printable() {
        let out = sanitise_label(&format!("desk\u{0}top{}", "x".repeat(200)));
        assert!(out.len() <= 48);
        assert!(!out.contains('\u{0}'));
    }
}
