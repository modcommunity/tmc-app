use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::plugins::manifest::{Manifest, MANIFEST_FILE};
use crate::plugins::signature::{self, SignatureState, TrustStore, TrustedKey};

/// What the app knows about the plugins on this machine.
///
/// The registry is the consent record, not just an index. Its central field is
/// `approved_fingerprint`: the hash of the manifest the user actually saw and
/// agreed to. A plugin whose manifest no longer hashes to that value is
/// disabled on sight and needs re-approval, which is what makes a silent
/// permission escalation — an update that adds a write grant or a new download
/// host — impossible to slip past.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: Option<String>,

    /// Which capabilities the manifest declares.
    pub kinds: Vec<String>,
    /// TMC app ids handled. Empty = any.
    pub apps: Vec<i64>,

    /// Human-readable permission list, exactly as shown at approval time.
    pub permissions: Vec<String>,

    /// SHA-256 of the manifest the user approved.
    pub approved_fingerprint: String,
    pub approved_at: String,

    pub enabled: bool,

    /// Where the bundle lives, relative to the plugins directory.
    pub dir: String,

    /// Set when the on-disk manifest stopped matching `approved_fingerprint`.
    #[serde(default)]
    pub needs_reapproval: bool,

    /// What the signature check said, last time the bundle was read.
    ///
    /// Recorded rather than only computed on demand so the plugin list can show
    /// it without re-reading every bundle, and re-computed on every rescan so a
    /// key removed from the trust store takes effect on the next launch.
    ///
    /// Defaulted for a registry written before signatures existed: those
    /// plugins read as unsigned, which is what they are.
    #[serde(default = "unsigned")]
    pub signature: SignatureState,
}

fn unsigned() -> SignatureState {
    SignatureState::Unsigned
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryFile {
    #[serde(default)]
    plugins: BTreeMap<String, PluginRecord>,
}

pub struct Registry {
    path: PathBuf,
    plugins_dir: PathBuf,
    state: RwLock<RegistryFile>,
    /// Parsed manifests, keyed by id. Rebuilt on load and on install.
    loaded: RwLock<BTreeMap<String, Manifest>>,

    /// Keys the user trusts. Re-read on every [`Registry::rescan`], so adding
    /// or removing one takes effect without a restart.
    trust: RwLock<TrustStore>,

    /// Whether an unsigned plugin may run at all.
    ///
    /// A copy of `AppSettings::require_signed_plugins`, pushed in rather than
    /// read here: this crate's registry has no settings store, and threading
    /// one through would make the gate depend on load order. The app sets it at
    /// startup and on every settings change.
    require_signed: AtomicBool,
}

impl Registry {
    pub fn load(path: PathBuf, plugins_dir: PathBuf) -> Self {
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<RegistryFile>(&raw).ok())
            .unwrap_or_default();

        let registry = Self {
            trust: RwLock::new(TrustStore::load(&plugins_dir).unwrap_or_else(|e| {
                // A trust store that will not parse must not stop the app from
                // starting. It is reported and treated as empty for this run,
                // which fails CLOSED — every plugin reads as untrusted rather
                // than every plugin reading as fine.
                tracing::warn!("trusted-keys.json could not be read: {e}");

                TrustStore::default()
            })),
            path,
            plugins_dir,
            state: RwLock::new(state),
            loaded: RwLock::new(BTreeMap::new()),
            require_signed: AtomicBool::new(false),
        };

        registry.rescan();

        registry
    }

    /// Point the gate at the user's current setting.
    ///
    /// Called at startup and whenever settings change. Separate from `load`
    /// because the settings store and the registry are both built in
    /// `AppState::build` and neither can read the other.
    pub fn set_require_signed(&self, required: bool) {
        self.require_signed.store(required, Ordering::Relaxed);
    }

    pub fn require_signed(&self) -> bool {
        self.require_signed.load(Ordering::Relaxed)
    }

    /// Every key the user trusts.
    pub fn trusted_keys(&self) -> Vec<TrustedKey> {
        self.trust.read().map(|t| t.list()).unwrap_or_default()
    }

    /// Trust a key, and re-check every installed plugin against it.
    ///
    /// The rescan is the point: adding the key a plugin was signed with should
    /// make that plugin usable immediately, not after a restart.
    pub fn trust_key(&self, id: &str, label: &str, public_key: &str) -> AppResult<TrustedKey> {
        let added = {
            let mut store = self
                .trust
                .write()
                .map_err(|_| AppError::internal("trust store lock poisoned"))?;

            let added = store.add(id, label, public_key)?;

            store.save(&self.plugins_dir)?;

            added
        };

        self.rescan();

        Ok(added)
    }

    /// Stop trusting a key.
    ///
    /// Plugins it signed become `Untrusted` on the rescan — and stop running
    /// immediately if signatures are required, which is the entire reason to
    /// remove a key.
    pub fn untrust_key(&self, id: &str) -> AppResult<bool> {
        let removed = {
            let mut store = self
                .trust
                .write()
                .map_err(|_| AppError::internal("trust store lock poisoned"))?;

            let removed = store.remove(id);

            if removed {
                store.save(&self.plugins_dir)?;
            }

            removed
        };

        if removed {
            self.rescan();
        }

        Ok(removed)
    }

    /// Re-read every recorded plugin's manifest from disk and re-check its
    /// fingerprint.
    ///
    /// Runs on every launch, not only on install. The bundle is a directory in
    /// the user's data dir — another process, a sync client, or the plugin's
    /// own updater can change it between runs, and the approval has to be
    /// re-verified against what is there NOW.
    pub fn rescan(&self) {
        let records: Vec<PluginRecord> = self
            .state
            .read()
            .map(|s| s.plugins.values().cloned().collect())
            .unwrap_or_default();

        // Re-read, so a key added or removed since launch is in effect.
        if let Ok(store) = TrustStore::load(&self.plugins_dir) {
            if let Ok(mut trust) = self.trust.write() {
                *trust = store;
            }
        }

        let mut loaded = BTreeMap::new();
        let mut changed: Vec<(String, bool, SignatureState)> = Vec::new();

        for record in records {
            let dir = self.plugins_dir.join(&record.dir);
            let manifest_path = dir.join(MANIFEST_FILE);

            let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
                changed.push((record.id.clone(), true, SignatureState::Unsigned));
                continue;
            };

            match Manifest::parse(&raw) {
                Ok(manifest) => {
                    let drifted = manifest.fingerprint() != record.approved_fingerprint;

                    if drifted {
                        tracing::warn!(
                            plugin = %record.id,
                            "manifest changed since approval; disabling"
                        );
                    }

                    /*
                     * Over the CANONICAL bytes, which is what was signed and
                     * what the fingerprint covers — not over `raw`. A bundle
                     * reformatted on the way here keeps its signature, and a
                     * bundle whose meaning changed loses it.
                     */
                    let signature = self.signature_of(&dir, &manifest);

                    changed.push((record.id.clone(), drifted, signature));
                    loaded.insert(record.id.clone(), manifest);
                }
                Err(e) => {
                    tracing::warn!(plugin = %record.id, "manifest no longer valid: {e}");
                    changed.push((record.id.clone(), true, SignatureState::Unsigned));
                }
            }
        }

        if let Ok(mut state) = self.state.write() {
            for (id, needs, signature) in changed {
                if let Some(record) = state.plugins.get_mut(&id) {
                    record.needs_reapproval = needs;
                    record.signature = signature;

                    // A drifted plugin does not run until re-approved. Not a
                    // prompt-on-next-use: the install step executor must never
                    // be reachable with an unapproved manifest.
                    if needs {
                        record.enabled = false;
                    }
                }
            }
        }

        if let Ok(mut slot) = self.loaded.write() {
            *slot = loaded;
        }

        let _ = self.persist();
    }

    pub fn list(&self) -> Vec<PluginRecord> {
        self.state
            .read()
            .map(|s| s.plugins.values().cloned().collect())
            .unwrap_or_default()
    }

    /// The manifest for a plugin that is installed, approved and enabled.
    ///
    /// The only accessor the executor uses. Every reason a plugin must not run
    /// is checked here, once, rather than at each call site.
    pub fn active(&self, id: &str) -> AppResult<Manifest> {
        let record = self
            .state
            .read()
            .ok()
            .and_then(|s| s.plugins.get(id).cloned())
            .ok_or_else(|| AppError::invalid("That plugin is not installed."))?;

        if record.needs_reapproval {
            return Err(AppError::jail(
                "This plugin changed since you approved it. Review it again in Settings → Plugins.",
            ));
        }

        if !record.enabled {
            return Err(AppError::invalid("That plugin is disabled."));
        }

        let manifest = self
            .loaded
            .read()
            .ok()
            .and_then(|m| m.get(id).cloned())
            .ok_or_else(|| AppError::invalid("That plugin could not be loaded."))?;

        // Belt and braces: `rescan` already compared these, but this is the
        // gate in front of arbitrary filesystem writes, so it re-checks rather
        // than trusting state written by another code path.
        if manifest.fingerprint() != record.approved_fingerprint {
            return Err(AppError::jail(
                "This plugin's manifest does not match what you approved.",
            ));
        }

        /*
         * And the same for the signature, when the user has asked for one.
         *
         * Re-verified here rather than read from `record.signature`, for
         * exactly the reason above: this is the last gate before an executor
         * writes to somebody's game folder, and a stored boolean is a claim
         * about a check that happened at some earlier time under a trust store
         * that may since have changed.
         */
        if self.require_signed() {
            let dir = self.plugins_dir.join(&record.dir);

            match self.signature_of(&dir, &manifest) {
                SignatureState::Trusted(_) => {}
                SignatureState::Untrusted => {
                    return Err(AppError::jail(
                        "This plugin is signed by a key you have not trusted, and this app is set \
                         to run signed plugins only. Add the publisher's key in Settings → \
                         Plugins, or turn the requirement off.",
                    ))
                }
                SignatureState::Unsigned => {
                    return Err(AppError::jail(
                        "This plugin is not signed, and this app is set to run signed plugins \
                         only. Turn the requirement off in Settings → Plugins to use it.",
                    ))
                }
            }
        }

        Ok(manifest)
    }

    /// Check one bundle's signature over the manifest that was loaded from it.
    ///
    /// Over the CANONICAL bytes — the same serialisation `fingerprint` hashes —
    /// so a bundle reformatted in transit keeps its signature and a bundle
    /// whose declared permissions changed loses it.
    fn signature_of(&self, dir: &Path, manifest: &Manifest) -> SignatureState {
        let canonical = match serde_json::to_vec(manifest) {
            Ok(bytes) => bytes,
            // Cannot happen for a parsed manifest, and if it somehow did, the
            // safe answer is "nobody vouched for this".
            Err(_) => return SignatureState::Unsigned,
        };

        let Ok(trust) = self.trust.read() else {
            return SignatureState::Unsigned;
        };

        match signature::check(dir, &canonical, &trust) {
            Ok(state) => state,
            Err(e) => {
                /*
                 * A signature file that is present and unreadable is NOT
                 * "unsigned" — it is a bundle making a claim that does not
                 * check out, which is the more interesting of the two. It reads
                 * as untrusted, so a build requiring signatures refuses it.
                 */
                tracing::warn!(plugin = %manifest.id, "signature could not be read: {e}");

                SignatureState::Untrusted
            }
        }
    }

    /// Read and validate a bundle without installing it, for the consent
    /// dialog. Nothing is written and nothing becomes runnable.
    pub fn inspect(&self, dir: &Path) -> AppResult<(Manifest, Vec<String>, SignatureState)> {
        let raw = std::fs::read_to_string(dir.join(MANIFEST_FILE))
            .map_err(|_| AppError::invalid("No plugin.json in that folder."))?;

        let manifest = Manifest::parse(&raw)?;
        let summary = manifest.permission_summary();

        // Shown in the approval dialog beside the permissions. Who vouched for
        // a plugin is part of what somebody is deciding about, and finding out
        // afterwards from a list is finding out too late.
        let signature = self.signature_of(dir, &manifest);

        Ok((manifest, summary, signature))
    }

    /// Record the user's approval and enable the plugin.
    ///
    /// `expected_fingerprint` is what the dialog displayed. Passing it back
    /// closes the window between "user reads the permissions" and "user clicks
    /// approve": if the manifest changed in between, the fingerprints disagree
    /// and nothing is installed.
    pub fn approve(
        &self,
        manifest: &Manifest,
        source_dir: &Path,
        expected_fingerprint: &str,
    ) -> AppResult<PluginRecord> {
        let fingerprint = manifest.fingerprint();

        if fingerprint != expected_fingerprint {
            return Err(AppError::jail(
                "The plugin changed while you were reviewing it. Nothing was installed.",
            ));
        }

        let dest = self.plugins_dir.join(&manifest.id);

        if source_dir != dest {
            if dest.exists() {
                std::fs::remove_dir_all(&dest)?;
            }

            copy_bundle(source_dir, &dest)?;
        }

        let mut kinds = Vec::new();
        if manifest.installer.is_some() {
            kinds.push("installer".to_string());
        }
        if manifest.server_query.is_some() {
            kinds.push("serverQuery".to_string());
        }
        if manifest.theme.is_some() {
            kinds.push("theme".to_string());
        }
        if manifest.manager.is_some() {
            kinds.push("manager".to_string());
        }

        let record = PluginRecord {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            author: manifest.author.clone(),
            description: manifest.description.clone(),
            kinds,
            apps: manifest.apps.clone(),
            permissions: manifest.permission_summary(),
            approved_fingerprint: fingerprint,
            approved_at: crate::logging::now_rfc3339(),
            enabled: true,
            dir: manifest.id.clone(),
            needs_reapproval: false,
            // From the INSTALLED copy, not the source folder: that is the one
            // every later check reads.
            signature: self.signature_of(&dest, manifest),
        };

        if let Ok(mut state) = self.state.write() {
            state.plugins.insert(manifest.id.clone(), record.clone());
        }

        if let Ok(mut loaded) = self.loaded.write() {
            loaded.insert(manifest.id.clone(), manifest.clone());
        }

        self.persist()?;

        Ok(record)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> AppResult<()> {
        {
            let mut state = self
                .state
                .write()
                .map_err(|_| AppError::internal("registry lock poisoned"))?;

            let record = state
                .plugins
                .get_mut(id)
                .ok_or_else(|| AppError::invalid("That plugin is not installed."))?;

            // Enabling a drifted plugin has to go through approval, not a
            // toggle — the toggle is not where the permissions are shown.
            if enabled && record.needs_reapproval {
                return Err(AppError::jail(
                    "Review this plugin's permissions again before enabling it.",
                ));
            }

            record.enabled = enabled;
        }

        self.persist()
    }

    pub fn remove(&self, id: &str) -> AppResult<()> {
        let dir = {
            let mut state = self
                .state
                .write()
                .map_err(|_| AppError::internal("registry lock poisoned"))?;

            state.plugins.remove(id).map(|r| r.dir)
        };

        if let Ok(mut loaded) = self.loaded.write() {
            loaded.remove(id);
        }

        if let Some(dir) = dir {
            // `dir` is always the plugin id, which `is_valid_plugin_id` proved
            // is a single safe component — but this is a recursive delete, so
            // it is re-derived rather than trusted from the file.
            if crate::plugins::manifest::is_valid_plugin_id(&dir) {
                let _ = std::fs::remove_dir_all(self.plugins_dir.join(&dir));
            }
        }

        self.persist()
    }

    /// The enabled theme plugin's tokens, if the given id names one.
    pub fn theme_tokens(&self, id: &str) -> Option<crate::plugins::manifest::Theme> {
        let manifest = self.active(id).ok()?;

        manifest.theme.clone()
    }

    fn persist(&self) -> AppResult<()> {
        let state = self
            .state
            .read()
            .map_err(|_| AppError::internal("registry lock poisoned"))?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&*state)?)?;
        std::fs::rename(&tmp, &self.path)?;

        Ok(())
    }
}

/// Cap on a plugin bundle. Manifests, a README, an icon — not game assets.
const MAX_BUNDLE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_BUNDLE_FILES: usize = 256;

/// Copy a bundle into the plugins directory, bounded and flat-ish.
///
/// Symlinks are skipped rather than followed: a bundle containing
/// `data -> /home/user` would otherwise be copied wholesale into the app's
/// data directory, or — worse — leave a live link behind that later jail
/// resolution has to catch.
fn copy_bundle(from: &Path, to: &Path) -> AppResult<()> {
    let mut budget = MAX_BUNDLE_BYTES;
    let mut files = 0usize;

    fn walk(
        from: &Path,
        to: &Path,
        budget: &mut u64,
        files: &mut usize,
        depth: u8,
    ) -> AppResult<()> {
        if depth > 8 {
            return Err(AppError::invalid("Plugin folder is nested too deeply."));
        }

        std::fs::create_dir_all(to)?;

        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let meta = entry.metadata()?;

            if meta.file_type().is_symlink() {
                continue;
            }

            let name = entry.file_name();
            let dest = to.join(&name);

            if meta.is_dir() {
                walk(&entry.path(), &dest, budget, files, depth + 1)?;
                continue;
            }

            *files += 1;

            if *files > MAX_BUNDLE_FILES {
                return Err(AppError::invalid("Plugin folder has too many files."));
            }

            *budget = budget
                .checked_sub(meta.len())
                .ok_or_else(|| AppError::invalid("Plugin folder is too large."))?;

            std::fs::copy(entry.path(), &dest)?;
        }

        Ok(())
    }

    walk(from, to, &mut budget, &mut files, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::plugins::signature::{SIGNATURE_FILE, TRUSTED_KEYS_FILE};
    use ed25519_dalek::{Signer, SigningKey};

    /// A deterministic publisher. No RNG feature is taken by this crate, and a
    /// fixed key makes a failure reproducible.
    fn signer(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    const MANIFEST: &str = r#"{
        "manifestVersion": 1,
        "id": "com.example.thing",
        "name": "Thing",
        "version": "1.0",
        "author": "Example",
        "permissions": { "fs": [{ "root": "pluginData", "path": "", "write": true }] },
        "installer": { "install": [], "uninstall": [] }
    }"#;

    struct Fixture {
        _tmp: tempfile::TempDir,
        plugins: PathBuf,
        source: PathBuf,
        registry: Registry,
    }

    /// A bundle on disk plus a registry pointed at an empty plugins directory.
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().expect("tempdir");

        let plugins = tmp.path().join("plugins");
        let source = tmp.path().join("source");

        std::fs::create_dir_all(&plugins).expect("mkdir");
        std::fs::create_dir_all(&source).expect("mkdir");
        std::fs::write(source.join(MANIFEST_FILE), MANIFEST).expect("write");

        let registry = Registry::load(tmp.path().join("registry.json"), plugins.clone());

        Fixture {
            _tmp: tmp,
            plugins,
            source,
            registry,
        }
    }

    /// Sign the bundle the way a publisher would: over the manifest's canonical
    /// bytes, which is what `fingerprint` hashes and what the app checks.
    fn sign(dir: &Path, seed: u8) {
        let raw = std::fs::read_to_string(dir.join(MANIFEST_FILE)).expect("read");
        let manifest = Manifest::parse(&raw).expect("parse");
        let canonical = serde_json::to_vec(&manifest).expect("canonical");

        std::fs::write(
            dir.join(SIGNATURE_FILE),
            hex::encode(signer(seed).sign(&canonical).to_bytes()),
        )
        .expect("write");
    }

    fn install(fx: &Fixture) -> Manifest {
        let (manifest, _summary, _signature) = fx.registry.inspect(&fx.source).expect("inspect");
        let fingerprint = manifest.fingerprint();

        fx.registry
            .approve(&manifest, &fx.source, &fingerprint)
            .expect("approve");

        manifest
    }

    #[test]
    fn an_unsigned_plugin_runs_when_signatures_are_not_required() {
        let fx = fixture();

        install(&fx);

        assert!(fx.registry.active("com.example.thing").is_ok());
    }

    /// The setting has to actually do something. It was a stored boolean with
    /// nothing behind it, which is worse than not having it: somebody turns it
    /// on, believes they are protected, and installs accordingly.
    #[test]
    fn an_unsigned_plugin_is_refused_when_signatures_are_required() {
        let fx = fixture();

        install(&fx);
        fx.registry.set_require_signed(true);

        let err = fx
            .registry
            .active("com.example.thing")
            .expect_err("must refuse");

        assert!(
            err.to_string().contains("not signed"),
            "the refusal has to say which problem it is: {err}"
        );
    }

    #[test]
    fn a_plugin_signed_by_a_trusted_key_runs_with_the_requirement_on() {
        let fx = fixture();

        sign(&fx.source, 1);
        install(&fx);

        fx.registry
            .trust_key("pub", "Example", &hex::encode(signer(1).verifying_key()))
            .expect("trust");

        fx.registry.set_require_signed(true);

        assert!(fx.registry.active("com.example.thing").is_ok());
    }

    /// "Signed by somebody you have not trusted" and "not signed at all" are
    /// different problems with different fixes, so they get different messages.
    #[test]
    fn an_untrusted_signature_is_refused_and_says_so_differently() {
        let fx = fixture();

        sign(&fx.source, 9);
        install(&fx);
        fx.registry.set_require_signed(true);

        let err = fx
            .registry
            .active("com.example.thing")
            .expect_err("must refuse");

        assert!(
            err.to_string().contains("have not trusted"),
            "an untrusted signature is not the same as no signature: {err}"
        );
    }

    /// Trusting a key takes effect immediately. Requiring a restart would make
    /// "add the publisher's key" advice that does not appear to work.
    #[test]
    fn trusting_a_key_makes_its_plugins_usable_without_a_restart() {
        let fx = fixture();

        sign(&fx.source, 1);
        install(&fx);
        fx.registry.set_require_signed(true);

        assert!(fx.registry.active("com.example.thing").is_err());

        fx.registry
            .trust_key("pub", "Example", &hex::encode(signer(1).verifying_key()))
            .expect("trust");

        assert!(fx.registry.active("com.example.thing").is_ok());

        // And removing it stops them again, which is the whole reason to be
        // able to remove one.
        assert!(fx.registry.untrust_key("pub").expect("untrust"));
        assert!(fx.registry.active("com.example.thing").is_err());
    }

    /// The gate re-verifies rather than reading the stored verdict: a trust
    /// store that changed since the last rescan must not leave a stale "this
    /// was fine" in front of the step executor.
    #[test]
    fn the_gate_rechecks_rather_than_trusting_the_recorded_verdict() {
        let fx = fixture();

        sign(&fx.source, 1);

        fx.registry
            .trust_key("pub", "Example", &hex::encode(signer(1).verifying_key()))
            .expect("trust");

        install(&fx);
        fx.registry.set_require_signed(true);

        assert!(fx.registry.active("com.example.thing").is_ok());

        /*
         * Remove the key from the FILE and reload only the store — no rescan,
         * so `record.signature` still says trusted. The gate must still refuse.
         */
        std::fs::write(fx.plugins.join(TRUSTED_KEYS_FILE), r#"{"keys":[]}"#).expect("write");

        {
            let mut trust = fx.registry.trust.write().expect("lock");
            *trust = TrustStore::load(&fx.plugins).expect("load");
        }

        assert_eq!(
            fx.registry
                .list()
                .iter()
                .find(|p| p.id == "com.example.thing")
                .map(|p| p.signature.clone()),
            Some(SignatureState::Trusted("pub".into())),
            "the recorded verdict is deliberately still the old one"
        );

        assert!(
            fx.registry.active("com.example.thing").is_err(),
            "and the gate refuses anyway"
        );
    }

    /// A signature does not excuse a manifest that changed after approval. The
    /// two checks are independent and both have to pass.
    #[test]
    fn a_signed_plugin_that_drifted_is_still_refused() {
        let fx = fixture();

        sign(&fx.source, 1);
        install(&fx);

        fx.registry
            .trust_key("pub", "Example", &hex::encode(signer(1).verifying_key()))
            .expect("trust");

        // Widen the installed copy's permissions behind the user's back, and
        // re-sign it so the signature itself is perfectly valid.
        let installed = fx.plugins.join("com.example.thing");

        std::fs::write(
            installed.join(MANIFEST_FILE),
            MANIFEST.replace(
                r#"{ "root": "pluginData", "path": "", "write": true }"#,
                r#"{ "root": "gameDir", "path": "", "write": true }"#,
            ),
        )
        .expect("write");

        sign(&installed, 1);

        fx.registry.rescan();

        let err = fx
            .registry
            .active("com.example.thing")
            .expect_err("must refuse");

        assert!(
            err.to_string().contains("approved"),
            "a valid signature over changed permissions is still not consent: {err}"
        );
    }

    /// A `plugin.sig` that is present and unreadable is not the same as none:
    /// it is a claim that does not check out, so it reads as untrusted.
    #[test]
    fn a_corrupt_signature_reads_as_untrusted_rather_than_unsigned() {
        let fx = fixture();

        std::fs::write(fx.source.join(SIGNATURE_FILE), "not a signature").expect("write");

        let (_manifest, _summary, state) = fx.registry.inspect(&fx.source).expect("inspect");

        assert_eq!(state, SignatureState::Untrusted);
    }

    #[test]
    fn the_approval_dialog_is_told_who_signed_it() {
        let fx = fixture();

        sign(&fx.source, 1);

        fx.registry
            .trust_key("pub", "Example", &hex::encode(signer(1).verifying_key()))
            .expect("trust");

        let (_manifest, summary, state) = fx.registry.inspect(&fx.source).expect("inspect");

        assert_eq!(state, SignatureState::Trusted("pub".into()));
        assert!(!summary.is_empty(), "and what it may do");
    }
}
