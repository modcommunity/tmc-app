//! Encrypting the few things this app has to keep that are not the user's to
//! lose and not ours to read.
//!
//! At the moment that is exactly one thing: **RCON passwords**. They are the
//! keys to somebody's game server, they cannot be hashed (the protocol needs
//! the password itself), and they must never leave the device — so they are
//! stored locally, encrypted, with the key in the platform's credential store.
//!
//! WHAT THIS DOES AND DOES NOT BUY
//! ------------------------------
//! The threat it answers is a **copied file**: a backup, a synced folder, a
//! stolen laptop's disk pulled and read, somebody else's process reading the
//! app's data directory. In all of those the database is readable and the
//! keychain is not, so the passwords stay closed.
//!
//! The threat it does **not** answer is code running as the user, in this
//! session, on this machine. That code can ask the keychain for the key exactly
//! as the app does. There is no way around that on any desktop platform and
//! claiming otherwise would be the dishonest part; what the design guarantees
//! instead is that the webview is not such code — nothing here is reachable
//! from JavaScript, and no command returns a decrypted password.
//!
//! THE CHOICES
//! -----------
//!   * **XChaCha20-Poly1305.** Authenticated, so a tampered ciphertext is an
//!     error rather than a wrong password sent to somebody's server. Chosen
//!     over AES-GCM because it does not need AES-NI to be fast, and two of the
//!     five platforms this ships to are ARM phones.
//!   * **A 192-bit random nonce, per message.** The reason for the X variant:
//!     at that width, generating one randomly has no birthday problem worth
//!     modelling, so there is no counter to persist and no way for a restore
//!     from backup to reuse one.
//!   * **The key never touches the database.** It lives in the OS credential
//!     store through [`crate::secure::SecureStore`], which falls back to the
//!     app's private container on mobile — where the OS provides the same
//!     guarantee at the filesystem layer.

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;

use crate::error::{AppError, AppResult};
use crate::secure::SecureStore;

/// The credential-store entry holding the key.
const KEY_NAME: &str = "local-secret-key";

/// XChaCha20's nonce width.
const NONCE_BYTES: usize = 24;

/// Refuse anything absurd before allocating for it.
const MAX_SEALED: usize = 64 * 1024;

/// A key, and the two operations it is for.
pub struct LocalCipher {
    cipher: XChaCha20Poly1305,
}

impl LocalCipher {
    /// Load the device's key, generating one the first time.
    ///
    /// Generating on first use rather than at install: a user who never adds an
    /// RCON server never has a key, so there is nothing to steal and nothing to
    /// migrate.
    pub fn load_or_create(secure: &SecureStore) -> AppResult<Self> {
        if let Some(stored) = secure.load_named(KEY_NAME) {
            if let Some(key) = decode_key(&stored) {
                return Ok(Self::from_key(&key));
            }

            /*
             * A key that will not decode is not something to fall back from
             * silently — everything sealed with the real one becomes
             * unreadable, and generating a replacement would turn "the entry is
             * corrupt" into "every saved password is wrong" with no message in
             * between.
             */
            return Err(AppError::internal(
                "the stored encryption key is unreadable",
            ));
        }

        let mut key = [0u8; 32];

        rand::thread_rng().fill_bytes(&mut key);

        secure.save_named(
            KEY_NAME,
            &base64::engine::general_purpose::STANDARD.encode(key),
        )?;

        Ok(Self::from_key(&key))
    }

    /// Build from raw key bytes. For tests, and for a caller that already holds
    /// one.
    pub fn from_key(key: &[u8; 32]) -> Self {
        Self {
            cipher: XChaCha20Poly1305::new(key.into()),
        }
    }

    /// Encrypt, returning `base64(nonce || ciphertext)`.
    pub fn seal(&self, plaintext: &str) -> AppResult<String> {
        let mut nonce = [0u8; NONCE_BYTES];

        rand::thread_rng().fill_bytes(&mut nonce);

        let ciphertext = self
            .cipher
            .encrypt(XNonce::from_slice(&nonce), plaintext.as_bytes())
            .map_err(|_| AppError::internal("encrypt failed"))?;

        let mut out = Vec::with_capacity(NONCE_BYTES + ciphertext.len());

        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);

        Ok(base64::engine::general_purpose::STANDARD.encode(out))
    }

    /// Decrypt what [`Self::seal`] produced.
    ///
    /// A failure here is not "wrong password" — it is a tampered or truncated
    /// record, or the wrong key entirely, and it must not be turned into an
    /// empty string that then gets sent to somebody's server as a login.
    pub fn open(&self, sealed: &str) -> AppResult<String> {
        if sealed.len() > MAX_SEALED {
            return Err(AppError::internal("sealed value is too large"));
        }

        let raw = base64::engine::general_purpose::STANDARD
            .decode(sealed)
            .map_err(|_| AppError::internal("sealed value is not base64"))?;

        if raw.len() <= NONCE_BYTES {
            return Err(AppError::internal("sealed value is truncated"));
        }

        let (nonce, ciphertext) = raw.split_at(NONCE_BYTES);

        let plaintext = self
            .cipher
            .decrypt(XNonce::from_slice(nonce), ciphertext)
            .map_err(|_| {
                AppError::internal("sealed value did not authenticate (wrong key, or tampered)")
            })?;

        String::from_utf8(plaintext).map_err(|_| AppError::internal("sealed value is not text"))
    }
}

fn decode_key(encoded: &str) -> Option<[u8; 32]> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;

    raw.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cipher() -> LocalCipher {
        LocalCipher::from_key(&[7u8; 32])
    }

    #[test]
    fn a_secret_round_trips() {
        let cipher = cipher();

        for secret in [
            "",
            "hunter2",
            "a password with spaces and ünïcödé",
            &"x".repeat(1000),
        ] {
            let sealed = cipher.seal(secret).expect("seal");

            assert_ne!(sealed, secret, "the stored form must not be the plaintext");
            assert_eq!(cipher.open(&sealed).expect("open"), secret);
        }
    }

    /// The reason for a random 192-bit nonce rather than a counter: two seals of
    /// the same value must not produce the same bytes, or the database leaks
    /// which servers share a password.
    #[test]
    fn sealing_the_same_value_twice_gives_different_ciphertext() {
        let cipher = cipher();

        let a = cipher.seal("hunter2").expect("seal");
        let b = cipher.seal("hunter2").expect("seal");

        assert_ne!(a, b);
        assert_eq!(cipher.open(&a).expect("open"), "hunter2");
        assert_eq!(cipher.open(&b).expect("open"), "hunter2");
    }

    /// Authenticated encryption, so a flipped bit is an error rather than a
    /// wrong password sent to somebody's server.
    #[test]
    fn a_tampered_value_fails_rather_than_decrypting_to_something() {
        let cipher = cipher();

        let sealed = cipher.seal("hunter2").expect("seal");

        let mut raw = base64::engine::general_purpose::STANDARD
            .decode(&sealed)
            .expect("decode");

        // Flip a bit in the ciphertext, past the nonce.
        let last = raw.len() - 1;
        raw[last] ^= 0x01;

        let tampered = base64::engine::general_purpose::STANDARD.encode(&raw);

        assert!(cipher.open(&tampered).is_err());

        // And in the nonce.
        raw[last] ^= 0x01;
        raw[0] ^= 0x01;

        let renonced = base64::engine::general_purpose::STANDARD.encode(&raw);

        assert!(cipher.open(&renonced).is_err());
    }

    #[test]
    fn another_key_cannot_open_it() {
        let sealed = cipher().seal("hunter2").expect("seal");

        let other = LocalCipher::from_key(&[9u8; 32]);

        assert!(other.open(&sealed).is_err());
    }

    #[test]
    fn junk_is_refused_rather_than_producing_an_empty_password() {
        let cipher = cipher();

        for bad in ["", "not base64!!", "aGVsbG8=", &"A".repeat(100_000)] {
            assert!(cipher.open(bad).is_err(), "{bad} should not open");
        }
    }

    #[test]
    fn a_key_is_created_once_and_reused() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let secure = SecureStore::new(tmp.path());

        let first = LocalCipher::load_or_create(&secure).expect("create");
        let sealed = first.seal("hunter2").expect("seal");

        // A second load must find the same key, or every saved password
        // silently stops working on the next launch.
        let second = LocalCipher::load_or_create(&secure).expect("load");

        assert_eq!(second.open(&sealed).expect("open"), "hunter2");
    }

    /// A corrupt key entry must say so rather than quietly minting a new key —
    /// which would turn "the entry is broken" into "every password is wrong"
    /// with nothing in between to explain it.
    #[test]
    fn a_corrupt_key_is_an_error_not_a_fresh_key() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let secure = SecureStore::new(tmp.path());

        secure
            .save_named("local-secret-key", "this is not a key")
            .expect("save");

        assert!(LocalCipher::load_or_create(&secure).is_err());
    }
}
