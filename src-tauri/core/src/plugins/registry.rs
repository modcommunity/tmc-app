use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::plugins::manifest::{Manifest, MANIFEST_FILE};

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
}

impl Registry {
    pub fn load(path: PathBuf, plugins_dir: PathBuf) -> Self {
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<RegistryFile>(&raw).ok())
            .unwrap_or_default();

        let registry = Self {
            path,
            plugins_dir,
            state: RwLock::new(state),
            loaded: RwLock::new(BTreeMap::new()),
        };

        registry.rescan();

        registry
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

        let mut loaded = BTreeMap::new();
        let mut changed: Vec<(String, bool)> = Vec::new();

        for record in records {
            let manifest_path = self.plugins_dir.join(&record.dir).join(MANIFEST_FILE);

            let Ok(raw) = std::fs::read_to_string(&manifest_path) else {
                changed.push((record.id.clone(), true));
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

                    changed.push((record.id.clone(), drifted));
                    loaded.insert(record.id.clone(), manifest);
                }
                Err(e) => {
                    tracing::warn!(plugin = %record.id, "manifest no longer valid: {e}");
                    changed.push((record.id.clone(), true));
                }
            }
        }

        if let Ok(mut state) = self.state.write() {
            for (id, needs) in changed {
                if let Some(record) = state.plugins.get_mut(&id) {
                    record.needs_reapproval = needs;

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
            return Err(AppError::sandbox(
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
            return Err(AppError::sandbox(
                "This plugin's manifest does not match what you approved.",
            ));
        }

        Ok(manifest)
    }

    /// Read and validate a bundle without installing it, for the consent
    /// dialog. Nothing is written and nothing becomes runnable.
    pub fn inspect(&self, dir: &Path) -> AppResult<(Manifest, Vec<String>)> {
        let raw = std::fs::read_to_string(dir.join(MANIFEST_FILE))
            .map_err(|_| AppError::invalid("No plugin.json in that folder."))?;

        let manifest = Manifest::parse(&raw)?;
        let summary = manifest.permission_summary();

        Ok((manifest, summary))
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
            return Err(AppError::sandbox(
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
                return Err(AppError::sandbox(
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
/// data directory, or — worse — leave a live link behind that later sandbox
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
