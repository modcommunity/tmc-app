use std::path::PathBuf;

use tauri::{AppHandle, Manager};

use tmc_core::error::{AppError, AppResult};

/// Where everything the app owns lives.
///
/// All of it comes from Tauri's path resolver rather than from `dirs` or a
/// hand-rolled `$HOME` join, because the answer differs per platform in ways
/// that matter: on Android and iOS these resolve inside the app's jailed
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

    /// Where every sandbox stages its mods, before deployment links or copies
    /// them into the game.
    ///
    /// In DATA, not cache: these are downloaded files a sandbox depends on, and
    /// an OS that cleared them would silently empty somebody's modlist. The
    /// download manager's own scratch space is the one that belongs in cache.
    pub fn staging_dir(&self) -> PathBuf {
        self.data.join("staging")
    }

    /// Where a deploy puts the game files it displaced, so a purge can put them
    /// back.
    ///
    /// The single most important directory in the app to not lose: it is the
    /// only copy of whatever a direct-strategy sandbox overwrote.
    pub fn backup_dir(&self) -> PathBuf {
        self.data.join("backups")
    }

    /// Where imported mods live — one directory per local mod, holding its
    /// files laid out as they belong under a game folder.
    ///
    /// In DATA for the same reason staging is, and one step stronger: an
    /// imported mod is very often the ONLY copy on the machine. Somebody drops
    /// a jar they were handed, deletes the download, and this is where it now
    /// lives. A cache directory the OS may clear is exactly the wrong home for
    /// that.
    ///
    /// Device-wide rather than per sandbox: one import is routinely in several
    /// sandboxes, and deployment only ever reads a mod's folder. See
    /// `tmc_core::local`.
    pub fn local_mods_dir(&self) -> PathBuf {
        self.data.join("local-mods")
    }

    /// The device's library database.
    ///
    /// In the DATA directory, not the cache: it records what is on disk in the
    /// user's game folders, and losing it would leave those files orphaned with
    /// nothing left that knows how to remove them. A cache directory is one the
    /// OS may clear whenever it likes.
    pub fn library_file(&self) -> PathBuf {
        self.data.join("library.sqlite3")
    }
}
