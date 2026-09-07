//! Who vouched for a plugin.
//!
//! The registry's manifest fingerprint answers *"is this the same plugin the
//! user approved?"*. It cannot answer *"did anybody the user trusts write
//! it?"* — a fingerprint is a hash of whatever was in the folder, so a bundle
//! dropped there by anything at all has a perfectly good one.
//!
//! This is the second question. A publisher signs the manifest's canonical
//! bytes with an Ed25519 key; the bundle carries the detached signature beside
//! it; the app checks it against keys the user has chosen to trust.
//!
//! WHY THIS EXISTS NOW
//! -------------------
//! `requireSignedPlugins` was a stored setting with nothing behind it. A switch
//! that names a security property and does not provide it is worse than no
//! switch: somebody turns it on, believes they are protected and installs
//! accordingly. It is either implemented or it is removed, and implementing it
//! is a hundred lines.
//!
//! THE TRUST STORE IS THE USER'S
//! -----------------------------
//! There is no key compiled into this binary. Trusted keys live in a file in
//! the plugins directory that the user adds to, and TMC's own publishing key
//! will be a row in it like anybody else's — which is the honest shape while
//! there is no plugin registry to distribute anything through, and stays
//! correct once there is.
//!
//! Not compiling one in is also what keeps this from being a lie in the other
//! direction: a bundled key nobody signs with would make every plugin
//! unsigned-but-shipped, and `requireSignedPlugins` a switch that refuses
//! everything.
//!
//! WHAT A SIGNATURE DOES AND DOES NOT BUY
//! --------------------------------------
//! It says a holder of that key produced *these permissions and these steps*.
//! It says nothing about whether the steps are safe — a signed plugin is
//! confined by exactly the same jail as an unsigned one, and this changes
//! nothing about what any plugin may reach. What it removes is the anonymous
//! bundle: a plugin that turns out to be hostile can be traced to a key, and
//! that key can be dropped from the store.
//!
//! **Hex, not base64.** Both values are fixed-length binary — 32 bytes of key,
//! 64 of signature — and hex is what `Manifest::fingerprint` already prints, so
//! there is one encoding in this crate rather than two. It also has no
//! alphabet or padding variants for somebody to pick the wrong one of.

use std::collections::BTreeMap;
use std::path::Path;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// The detached signature, beside `plugin.json` in the bundle.
pub const SIGNATURE_FILE: &str = "plugin.sig";

/// The user's trust store, in the plugins directory.
pub const TRUSTED_KEYS_FILE: &str = "trusted-keys.json";

/// Cap on the store. A user with more than this many publishing keys has a
/// different problem, and the file is read on every rescan.
const MAX_KEYS: usize = 64;

/// Cap on the signature file, which is 128 hex characters plus whitespace.
const MAX_SIGNATURE_BYTES: u64 = 1024;

/// One key the user has decided to trust.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedKey {
    /// Short, user-chosen, and what a plugin record stores — so a key can be
    /// rotated without every approval losing its meaning.
    pub id: String,
    /// What to show. The user typed this; it carries no authority.
    pub label: String,
    /// 32 bytes, hex.
    pub public_key: String,
    pub added_at: String,
}

/// Every key the user trusts, by id.
#[derive(Debug, Clone, Default)]
pub struct TrustStore {
    keys: BTreeMap<String, TrustedKey>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrustFile {
    #[serde(default)]
    keys: Vec<TrustedKey>,
}

impl TrustStore {
    /// Read the store, or an empty one.
    ///
    /// A missing file is the normal state and is not an error. A file that will
    /// not parse IS surfaced rather than silently ignored: this decides what
    /// runs on the user's machine, and quietly falling back to "trust nothing"
    /// would present as every plugin suddenly being unsigned.
    pub fn load(dir: &Path) -> AppResult<Self> {
        let path = dir.join(TRUSTED_KEYS_FILE);

        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(AppError::internal(format!(
                    "could not read {}: {e}",
                    path.display()
                )))
            }
        };

        let parsed: TrustFile = serde_json::from_str(&raw).map_err(|e| {
            AppError::invalid(format!("The trusted-keys file is not valid JSON: {e}"))
        })?;

        let mut keys = BTreeMap::new();

        for key in parsed.keys.into_iter().take(MAX_KEYS) {
            // A row whose key is malformed is DROPPED rather than failing the
            // load: one bad line must not take the other publishers with it.
            if decode_key(&key.public_key).is_err() || key.id.is_empty() {
                continue;
            }

            keys.insert(key.id.clone(), key);
        }

        Ok(Self { keys })
    }

    pub fn save(&self, dir: &Path) -> AppResult<()> {
        let file = TrustFile {
            keys: self.keys.values().cloned().collect(),
        };

        let body = serde_json::to_vec_pretty(&file)
            .map_err(|e| AppError::internal(format!("could not encode the trust store: {e}")))?;

        std::fs::create_dir_all(dir)
            .map_err(|e| AppError::internal(format!("could not create {}: {e}", dir.display())))?;

        // Written through a temporary and renamed: a half-written trust store
        // is one that silently stops trusting somebody.
        let path = dir.join(TRUSTED_KEYS_FILE);
        let tmp = path.with_extension("tmp");

        std::fs::write(&tmp, &body)
            .map_err(|e| AppError::internal(format!("could not write the trust store: {e}")))?;

        std::fs::rename(&tmp, &path)
            .map_err(|e| AppError::internal(format!("could not replace the trust store: {e}")))?;

        Ok(())
    }

    pub fn list(&self) -> Vec<TrustedKey> {
        self.keys.values().cloned().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Add a key, replacing one with the same id.
    ///
    /// The key's SHAPE is validated here — hex, 32 bytes — rather than at use,
    /// so a mistyped key is refused where it was typed. Whether those bytes are
    /// a real public key is not knowable until something is verified against
    /// them; see [`decode_key`] for why that is the safe direction.
    pub fn add(&mut self, id: &str, label: &str, public_key: &str) -> AppResult<TrustedKey> {
        let id = id.trim();

        if id.is_empty() || id.len() > 64 {
            return Err(AppError::invalid("A trusted key needs a short name."));
        }

        if self.keys.len() >= MAX_KEYS && !self.keys.contains_key(id) {
            return Err(AppError::invalid(
                "There are too many trusted keys already.",
            ));
        }

        let normalised = public_key.trim().to_ascii_lowercase();

        decode_key(&normalised)?;

        let key = TrustedKey {
            id: id.to_string(),
            label: label.trim().chars().take(96).collect(),
            public_key: normalised,
            added_at: crate::logging::now_rfc3339(),
        };

        self.keys.insert(key.id.clone(), key.clone());

        Ok(key)
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.keys.remove(id).is_some()
    }

    /// Which trusted key signed these bytes, if any.
    ///
    /// Every key is tried. There is no hint in the signature about which key
    /// produced it, and adding one — a key id in the `.sig` file — would be a
    /// value the bundle controls that decides which key gets checked.
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Option<&TrustedKey> {
        let Ok(bytes) = <[u8; 64]>::try_from(signature) else {
            return None;
        };

        let signature = Signature::from_bytes(&bytes);

        self.keys.values().find(|key| {
            decode_key(&key.public_key)
                .ok()
                .is_some_and(|verifying| verifying.verify(message, &signature).is_ok())
        })
    }
}

/// A hex public key as something that can verify.
fn decode_key(hex_key: &str) -> AppResult<VerifyingKey> {
    let raw = hex::decode(hex_key.trim())
        .map_err(|_| AppError::invalid("A public key must be hexadecimal."))?;

    let bytes = <[u8; 32]>::try_from(raw.as_slice())
        .map_err(|_| AppError::invalid("An Ed25519 public key is 32 bytes — 64 hex characters."))?;

    /*
     * What this catches is the shape: not hex, or not 32 bytes. It does NOT
     * establish that the bytes are a real public key — `VerifyingKey::from_bytes`
     * in dalek 2 defers decompression, so 32 bytes of anything constructs
     * successfully and only fails later, when a verification against it never
     * matches.
     *
     * That is the safe direction and it is worth being precise about rather
     * than claiming more: a typo in a key does not make anything trusted, it
     * makes that publisher's plugins read as `Untrusted` — which is the same
     * answer as an unknown signer, and is what the user should check first.
     */
    VerifyingKey::from_bytes(&bytes)
        .map_err(|_| AppError::invalid("That is not a valid Ed25519 public key."))
}

/// Read a bundle's detached signature, if it has one.
///
/// Absent is `Ok(None)` — most plugins are unsigned and that is a state to
/// report, not an error. Present but unreadable IS an error: a bundle carrying
/// a `plugin.sig` full of rubbish is a more interesting fact than one carrying
/// none, and treating the two the same hides it.
pub fn read_signature(dir: &Path) -> AppResult<Option<Vec<u8>>> {
    let path = dir.join(SIGNATURE_FILE);

    let meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(AppError::internal(format!(
                "could not read {}: {e}",
                path.display()
            )))
        }
    };

    if meta.len() > MAX_SIGNATURE_BYTES {
        return Err(AppError::invalid(
            "That plugin's signature file is not one.",
        ));
    }

    let raw = std::fs::read_to_string(&path)
        .map_err(|e| AppError::internal(format!("could not read {}: {e}", path.display())))?;

    let decoded = hex::decode(raw.trim())
        .map_err(|_| AppError::invalid("That plugin's signature is not hexadecimal."))?;

    if decoded.len() != 64 {
        return Err(AppError::invalid(
            "That plugin's signature is the wrong length.",
        ));
    }

    Ok(Some(decoded))
}

/// What a signature check concluded, for the record and for the dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state", content = "keyId")]
pub enum SignatureState {
    /// No `plugin.sig` in the bundle.
    Unsigned,
    /// Signed by a key in the trust store, named here.
    Trusted(String),
    /// A signature is present and matches nothing the user trusts.
    ///
    /// Deliberately NOT the same as unsigned. An unsigned plugin makes no
    /// claim; one carrying a signature nobody can check is either from a
    /// publisher the user has not added yet or is tampered with, and both are
    /// worth saying out loud rather than folding into "not signed".
    Untrusted,
}

impl SignatureState {
    pub fn is_trusted(&self) -> bool {
        matches!(self, Self::Trusted(_))
    }

    pub fn key_id(&self) -> Option<&str> {
        match self {
            Self::Trusted(id) => Some(id.as_str()),
            _ => None,
        }
    }
}

/// Check one bundle's signature over the manifest's canonical bytes.
///
/// The MANIFEST's bytes, not the bundle's, and canonical rather than as-written
/// — the same thing `Manifest::fingerprint` hashes. Signing the file as typed
/// would make a reformat break the signature; signing the whole bundle would
/// make it cover files the user never sees and never consents to. What is
/// signed is what the approval dialog displays.
pub fn check(dir: &Path, canonical: &[u8], store: &TrustStore) -> AppResult<SignatureState> {
    let Some(signature) = read_signature(dir)? else {
        return Ok(SignatureState::Unsigned);
    };

    Ok(match store.verify(canonical, &signature) {
        Some(key) => SignatureState::Trusted(key.id.clone()),
        None => SignatureState::Untrusted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use ed25519_dalek::{Signer, SigningKey};

    /// A deterministic key. `SigningKey::generate` needs an RNG feature this
    /// crate does not take, and a fixed key makes a failure reproducible.
    fn signer(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn store_with(seed: u8) -> TrustStore {
        let mut store = TrustStore::default();

        store
            .add(
                "tmc",
                "The Modding Community",
                &hex::encode(signer(seed).verifying_key().to_bytes()),
            )
            .expect("add");

        store
    }

    fn bundle(dir: &Path, signature: Option<&[u8]>) {
        std::fs::create_dir_all(dir).expect("mkdir");

        if let Some(signature) = signature {
            std::fs::write(dir.join(SIGNATURE_FILE), hex::encode(signature)).expect("write");
        }
    }

    #[test]
    fn a_signature_from_a_trusted_key_verifies() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("plugin");

        let message = br#"{"id":"com.example.thing"}"#;
        let signature = signer(1).sign(message);

        bundle(&dir, Some(&signature.to_bytes()));

        assert_eq!(
            check(&dir, message, &store_with(1)).expect("check"),
            SignatureState::Trusted("tmc".into())
        );
    }

    /// The distinction that matters: a bundle with no signature and a bundle
    /// with one nobody can check are different facts.
    #[test]
    fn an_unsigned_bundle_and_an_untrusted_one_are_not_the_same_answer() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let unsigned = tmp.path().join("unsigned");
        bundle(&unsigned, None);

        assert_eq!(
            check(&unsigned, b"anything", &store_with(1)).expect("check"),
            SignatureState::Unsigned
        );

        // Signed by a key the user does not trust.
        let stranger = tmp.path().join("stranger");
        let message = b"anything";

        bundle(&stranger, Some(&signer(9).sign(message).to_bytes()));

        assert_eq!(
            check(&stranger, message, &store_with(1)).expect("check"),
            SignatureState::Untrusted
        );
    }

    /// The whole point: change one byte of what was approved and the signature
    /// stops matching.
    #[test]
    fn a_signature_does_not_survive_an_edit_to_the_manifest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("plugin");

        let original = br#"{"permissions":{"fs":[]}}"#;

        bundle(&dir, Some(&signer(1).sign(original).to_bytes()));

        assert!(check(&dir, original, &store_with(1))
            .expect("check")
            .is_trusted());

        let widened = br#"{"permissions":{"fs":["everything"]}}"#;

        assert_eq!(
            check(&dir, widened, &store_with(1)).expect("check"),
            SignatureState::Untrusted,
            "a manifest that changed after signing must not stay trusted"
        );
    }

    #[test]
    fn a_malformed_signature_file_is_an_error_rather_than_unsigned() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("plugin");

        std::fs::create_dir_all(&dir).expect("mkdir");

        for junk in ["", "not hex", "aabb", &"aa".repeat(65)] {
            std::fs::write(dir.join(SIGNATURE_FILE), junk).expect("write");

            assert!(
                check(&dir, b"x", &store_with(1)).is_err(),
                "{junk:?} should not read as a signature"
            );
        }
    }

    #[test]
    fn a_public_key_is_validated_when_it_is_added_and_not_later() {
        let mut store = TrustStore::default();

        assert!(store.add("a", "A", "not hex").is_err());
        assert!(store.add("a", "A", "aabbcc").is_err(), "too short");
        assert!(store.add("a", "A", &"aa".repeat(33)).is_err(), "too long");
        assert!(store
            .add("", "A", &hex::encode(signer(1).verifying_key()))
            .is_err());

        assert!(store
            .add("a", "A", &hex::encode(signer(1).verifying_key()))
            .is_ok());

        /*
         * 32 bytes that are not a real key ARE accepted, and that is the
         * documented behaviour rather than a gap: dalek defers decompression,
         * so the wrongness only shows up as verification never matching. The
         * safe direction — a mistyped key trusts nothing — and the test says so
         * out loud, because the obvious assumption is the opposite.
         */
        assert!(store.add("junk", "A", &"ff".repeat(32)).is_ok());
        assert!(
            store
                .verify(b"anything", &signer(1).sign(b"anything").to_bytes())
                .is_some(),
            "and it does not stop the real key from matching"
        );
    }

    /// Keys are matched by trying all of them, so a store with several works
    /// and the answer names the one that actually signed it.
    #[test]
    fn the_matching_key_is_the_one_reported() {
        let mut store = TrustStore::default();

        for seed in [1u8, 2, 3] {
            store
                .add(
                    &format!("key{seed}"),
                    "k",
                    &hex::encode(signer(seed).verifying_key()),
                )
                .expect("add");
        }

        let message = b"a manifest";

        assert_eq!(
            store
                .verify(message, &signer(2).sign(message).to_bytes())
                .map(|k| k.id.as_str()),
            Some("key2"),
            "the key that signed it is the one reported"
        );

        assert!(store
            .verify(message, &signer(7).sign(message).to_bytes())
            .is_none());
    }

    #[test]
    fn the_store_round_trips_through_its_file() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let mut store = store_with(1);

        store
            .add(
                "other",
                "Someone Else",
                &hex::encode(signer(5).verifying_key()),
            )
            .expect("add");

        store.save(tmp.path()).expect("save");

        let read = TrustStore::load(tmp.path()).expect("load");

        assert_eq!(read.list().len(), 2);
        assert!(read.list().iter().any(|k| k.id == "other"));

        // Removing one persists too.
        let mut read = read;

        assert!(read.remove("other"));
        read.save(tmp.path()).expect("save");

        assert_eq!(TrustStore::load(tmp.path()).expect("load").list().len(), 1);
    }

    #[test]
    fn a_missing_store_is_empty_rather_than_an_error() {
        let tmp = tempfile::tempdir().expect("tempdir");

        assert!(TrustStore::load(tmp.path()).expect("load").is_empty());
    }

    /// One unusable row must not take the other publishers with it.
    #[test]
    fn a_bad_row_is_dropped_and_the_rest_survive() {
        let tmp = tempfile::tempdir().expect("tempdir");

        std::fs::write(
            tmp.path().join(TRUSTED_KEYS_FILE),
            format!(
                r#"{{"keys":[
                    {{"id":"bad","label":"x","publicKey":"nonsense","addedAt":"now"}},
                    {{"id":"good","label":"y","publicKey":"{}","addedAt":"now"}}
                ]}}"#,
                hex::encode(signer(1).verifying_key())
            ),
        )
        .expect("write");

        let store = TrustStore::load(tmp.path()).expect("load");

        assert_eq!(store.list().len(), 1);
        assert_eq!(store.list()[0].id, "good");
    }

    /// A store file that will not parse is surfaced, not swallowed. Silently
    /// trusting nothing would present as every plugin suddenly being unsigned.
    #[test]
    fn an_unparseable_store_is_an_error() {
        let tmp = tempfile::tempdir().expect("tempdir");

        std::fs::write(tmp.path().join(TRUSTED_KEYS_FILE), "{ not json").expect("write");

        assert!(TrustStore::load(tmp.path()).is_err());
    }
}
