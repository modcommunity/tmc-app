use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::AppResult;

/// Settings that belong to this INSTALL, not to the account.
///
/// The split is deliberate and the line is: *would this be wrong to apply on a
/// different machine?* A theme, a download directory, whether to minimise to
/// tray — yes, those are machine facts. Notification preferences and locale are
/// account facts and live on the website (`/api/app/v1/me`), so signing in on a
/// new device inherits them.
///
/// Nothing here is secret. Tokens live in [`crate::secure`]; putting even one
/// credential in this file would mean the whole thing could no longer be
/// exported, backed up or shown in a support thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    /// Bumped when a migration is needed. Read before anything else.
    pub version: u32,

    // ------------------------------------------------------------ Appearance
    /// `system`, `light`, `dark`, or `plugin:<id>` for a theme plugin.
    pub theme: String,
    /// Base font scale, 0.85–1.4. The mobile shells need this more than desktop.
    pub ui_scale: f32,
    /// Denser grids on large screens.
    pub compact_cards: bool,
    /// Show NSFW content in browsers. Off by default, and the browse request
    /// omits the flag entirely when off — the server's default is also "hide".
    pub show_nsfw: bool,

    // --------------------------------------------------------------- Privacy
    /// Whether the low-severity half of the activity log is recorded. Security
    /// events are always recorded — see [`crate::logging::Audit`].
    pub verbose_logging: bool,
    /// Send anonymous crash reports. Off until asked for.
    pub crash_reports: bool,

    // -------------------------------------------------------------- Behaviour
    /// Where installers put downloads before they are extracted.
    pub download_dir: Option<String>,
    /// Game install roots, keyed by TMC app id. An installer plugin can only
    /// ever write under the root for the app it is handling.
    pub game_dirs: std::collections::BTreeMap<String, String>,
    /// Check for app updates on launch.
    pub auto_update_check: bool,
    /// Ping servers in the browser as their rows come into view.
    pub live_latency: bool,
    /// Concurrent latency probes. Bounded — a grid of 50 servers should not
    /// open 50 sockets at once on a phone.
    pub latency_concurrency: u8,
    /// Keep running in the tray/background when the window is closed. Desktop
    /// only; ignored on mobile, where the OS owns the decision.
    pub minimise_to_tray: bool,

    // --------------------------------------------------------------- Plugins
    /// Refuse to run any plugin that is not signed by an approved key. Off by
    /// default because nothing is signed yet; the manifest-hash approval in
    /// [`crate::plugins::registry`] is the control that is always on.
    pub require_signed_plugins: bool,
    /// Ask before every install/uninstall run, even for an approved plugin.
    pub confirm_every_run: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            theme: "system".into(),
            ui_scale: 1.0,
            compact_cards: false,
            show_nsfw: false,
            verbose_logging: true,
            crash_reports: false,
            download_dir: None,
            game_dirs: Default::default(),
            auto_update_check: true,
            live_latency: true,
            latency_concurrency: 8,
            minimise_to_tray: false,
            require_signed_plugins: false,
            confirm_every_run: true,
        }
    }
}

impl AppSettings {
    /// Clamp anything a hand-edited file could get wrong.
    ///
    /// The file is user-writable by design, so these are not paranoia about an
    /// attacker — they are about a typo turning into an unusable UI or a
    /// thousand concurrent sockets.
    fn sanitise(&mut self) {
        self.ui_scale = self.ui_scale.clamp(0.85, 1.4);
        self.latency_concurrency = self.latency_concurrency.clamp(1, 32);

        let theme_ok = matches!(self.theme.as_str(), "system" | "light" | "dark")
            || self
                .theme
                .strip_prefix("plugin:")
                .is_some_and(crate::plugins::manifest::is_valid_plugin_id);

        if !theme_ok {
            self.theme = "system".into();
        }
    }
}

pub struct SettingsStore {
    path: PathBuf,
    current: RwLock<AppSettings>,
}

impl SettingsStore {
    /// Load, or start from defaults.
    ///
    /// A corrupt file is replaced rather than reported: settings are
    /// reconstructible and refusing to launch over a malformed JSON file is a
    /// worse outcome than losing preferences. The old file is kept as
    /// `settings.broken.json` so it is not silently destroyed.
    pub fn load(path: PathBuf) -> Self {
        let mut settings = match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<AppSettings>(&raw) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("settings unreadable ({e}); starting fresh");
                    let _ = std::fs::rename(&path, path.with_extension("broken.json"));
                    AppSettings::default()
                }
            },
            Err(_) => AppSettings::default(),
        };

        settings.sanitise();

        Self {
            path,
            current: RwLock::new(settings),
        }
    }

    pub fn get(&self) -> AppSettings {
        self.current.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Merge a partial patch and persist.
    ///
    /// Takes JSON rather than a typed struct so the UI can send only what
    /// changed: two settings screens open at once must not have one clobber the
    /// other's field with a stale full object.
    pub fn patch(&self, patch: serde_json::Value) -> AppResult<AppSettings> {
        let mut merged = serde_json::to_value(self.get())?;

        if let (Some(base), Some(over)) = (merged.as_object_mut(), patch.as_object()) {
            for (k, v) in over {
                base.insert(k.clone(), v.clone());
            }
        }

        let mut next: AppSettings = serde_json::from_value(merged)?;
        next.sanitise();

        self.write(&next)?;

        if let Ok(mut cur) = self.current.write() {
            *cur = next.clone();
        }

        Ok(next)
    }

    /// Write via a temp file and rename.
    ///
    /// A settings file half-written by a crash or a full disk is the one thing
    /// that makes the app unlaunchable, and `rename` is atomic on every target
    /// platform's filesystem.
    fn write(&self, settings: &AppSettings) -> AppResult<()> {
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
        std::fs::rename(&tmp, &self.path)?;

        Ok(())
    }

    pub fn reset(&self) -> AppResult<AppSettings> {
        let fresh = AppSettings::default();
        self.write(&fresh)?;

        if let Ok(mut cur) = self.current.write() {
            *cur = fresh.clone();
        }

        Ok(fresh)
    }
}
