use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use tmc_core::error::{AppError, AppResult};

/// Where everything the app owns lives.
///
/// All of it comes from Tauri's path resolver rather than from `dirs` or a
/// hand-rolled `$HOME` join, because the answer differs per platform in ways
/// that matter: on Android and iOS these resolve inside the app's sandboxed
/// container, which is the only writable location and the only one the OS
/// isolates from other apps. A hardcoded `~/.tmc` would be both wrong and
/// world-readable there.
pub struct AppPaths {
    /// Settings, tokens (mobile fallback), plugin registry.
    pub data: PathBuf,
    /// The audit log.
    pub logs: PathBuf,
    /// Installed plugin bundles, one directory per plugin id.
    pub plugins: PathBuf,
    /// Scratch space for downloads mid-install. Cleared on launch.
    pub cache: PathBuf,
}

impl AppPaths {
    pub fn resolve(app: &AppHandle) -> AppResult<Self> {
        let resolver = app.path();

        let data = resolver
            .app_data_dir()
            .map_err(|e| AppError::internal(format!("no data dir: {e}")))?;
        let logs = resolver
            .app_log_dir()
            .map_err(|e| AppError::internal(format!("no log dir: {e}")))?;
        let cache = resolver
            .app_cache_dir()
            .map_err(|e| AppError::internal(format!("no cache dir: {e}")))?;

        let plugins = data.join("plugins");

        for dir in [&data, &logs, &cache, &plugins] {
            std::fs::create_dir_all(dir)?;
        }

        Ok(Self {
            data,
            logs,
            plugins,
            cache,
        })
    }

    pub fn settings_file(&self) -> PathBuf {
        self.data.join("settings.json")
    }

    pub fn audit_file(&self) -> PathBuf {
        self.logs.join("activity.jsonl")
    }

    pub fn registry_file(&self) -> PathBuf {
        self.data.join("plugins.json")
    }
}
