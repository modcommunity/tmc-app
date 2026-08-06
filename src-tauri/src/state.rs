use std::sync::Arc;

use tauri::AppHandle;

use tmc_core::api::ApiClient;
use tmc_core::auth::AuthState;
use tmc_core::error::AppResult;
use tmc_core::logging::Audit;
use tmc_core::net::LatencyStore;
use tmc_core::plugins::registry::Registry;
use tmc_core::plugins::SandboxRoots;
use tmc_core::secure::SecureStore;
use tmc_core::settings::SettingsStore;

use crate::paths::AppPaths;

/// Everything the commands need, assembled once at startup.
///
/// Held behind `tauri::State`, so every command gets the same instances — which
/// matters for the ones holding locks (the refresh mutex, the audit writer) or
/// caches (the plugin registry, the latency history). Constructing per-call
/// would quietly break the single-refresh guarantee in [`ApiClient`] and reset
/// every latency graph on each render.
pub struct AppState {
    pub paths: AppPaths,
    pub settings: SettingsStore,
    pub audit: Arc<Audit>,
    pub secure: Arc<SecureStore>,
    pub auth: Arc<AuthState>,
    pub api: ApiClient,
    pub plugins: Registry,
    pub latency: LatencyStore,
    pub version: String,
}

impl AppState {
    pub fn build(app: &AppHandle, version: String) -> AppResult<Self> {
        let paths = AppPaths::resolve(app)?;

        let settings = SettingsStore::load(paths.settings_file());
        let current = settings.get();

        let audit = Arc::new(Audit::new(paths.audit_file()));
        audit.set_verbose(current.verbose_logging);

        let secure = Arc::new(SecureStore::new(&paths.data));

        let auth = Arc::new(AuthState::new());
        auth.restore(&secure);

        let api = ApiClient::new(Arc::clone(&auth), Arc::clone(&secure), &version)?;

        let plugins = Registry::load(paths.registry_file(), paths.plugins.clone());

        Ok(Self {
            paths,
            settings,
            audit,
            secure,
            auth,
            api,
            plugins,
            latency: LatencyStore::new(),
            version,
        })
    }

    /// The directories a plugin sandbox may be anchored to.
    ///
    /// Resolved here rather than in `tmc-core` because the answer is
    /// platform-specific and Tauri's resolver is the only thing that knows it.
    /// The core decides what a plugin may *do* with them.
    pub fn sandbox_roots(&self) -> SandboxRoots {
        SandboxRoots {
            data: self.paths.data.clone(),
            cache: self.paths.cache.clone(),
        }
    }
}
