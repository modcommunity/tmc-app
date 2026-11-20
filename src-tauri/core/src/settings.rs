use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

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
    /// Game install roots, keyed by GameSpice app id. An installer plugin can only
    /// ever write under the root for the app it is handling.
    pub game_dirs: std::collections::BTreeMap<String, String>,
    /// Check for app updates on launch.
    pub auto_update_check: bool,
    /// Which release channel the app's own updater reads: `stable` or `beta`.
    /// A preference, and patchable — it chooses between two manifests the
    /// release workflow signs, never which signatures are trusted.
    pub update_channel: String,
    /// Where the app's own updates are fetched from, overriding the one
    /// compiled in. **Not patchable** — see [`UPDATER_SOURCE_FIELDS`].
    pub updater_endpoint: Option<String>,
    /// The minisign public key updates must be signed with, overriding the
    /// one compiled in. **Not patchable** — see [`UPDATER_SOURCE_FIELDS`].
    pub updater_pubkey: Option<String>,
    /// Ping servers in the browser as their rows come into view.
    pub live_latency: bool,
    /// How often the visible set is re-queried, in milliseconds.
    ///
    /// A second by default: a server browser's whole claim is that its numbers
    /// are current, and a player watching a filling server wants to see it fill.
    /// Clamped rather than free — see [`AppSettings::sanitise`] for the floor,
    /// which is the one value here that a typo could turn into a flood of
    /// datagrams at somebody else's server.
    pub latency_interval_ms: u32,
    /// Concurrent latency probes. Bounded — a grid of 50 servers should not
    /// open 50 sockets at once on a phone.
    pub latency_concurrency: u8,
    /// Keep running in the tray/background when the window is closed. Desktop
    /// only; ignored on mobile, where the OS owns the decision.
    pub minimise_to_tray: bool,

    // ------------------------------------------------------------- Downloads
    /// Ceiling across every download, in bytes per second. `0` is unlimited.
    ///
    /// A setting rather than a session value because the reason somebody sets
    /// one — a shared connection, a metered link, a housemate on a call — does
    /// not end when the app closes.
    pub download_limit_bps: u64,
    /// How many transfers run at once. Clamped to `download::MAX_CONCURRENT`.
    pub download_concurrency: u8,
    /// Keep finished downloads in the list until they are cleared.
    pub download_keep_history: bool,

    // --------------------------------------------------------------- Plugins
    /// Refuse to run any plugin that is not signed by an approved key. Off by
    /// default because nothing is signed yet; the manifest-hash approval in
    /// [`crate::plugins::registry`] is the control that is always on.
    pub require_signed_plugins: bool,
    /// Ask before every install/uninstall run, even for an approved plugin.
    pub confirm_every_run: bool,

    /// Allow the app to open a game's own settings files for editing.
    ///
    /// ON, because it is a mod manager and editing a loader's `.cfg` is
    /// ordinary work. It is here at all — rather than being unconditional —
    /// because it is the one feature that lets the WEBVIEW name a file to be
    /// written, and somebody who would rather that surface did not exist on
    /// their machine should be able to say so.
    ///
    /// What turning it off buys is bounded and worth stating plainly: it
    /// removes those three commands, and nothing else. It does not sandbox the
    /// process, which plainly writes to game folders — the deployment engine is
    /// what that is for. Every game's config locations are still declared by
    /// its own plugin and still resolved through the jail whether this is on or
    /// off; this only decides whether the door exists.
    pub allow_config_editing: bool,

    /// Whether a TMC game keeps what it downloads between sessions. Off, the
    /// shell downloads into a folder it empties at launch and at quit.
    pub game_cache_keep: bool,
    /// Ceiling for kept game downloads, in MiB. `0` leaves the game's own (4 GiB).
    pub game_cache_limit_mb: u32,
    /// Whether a TMC game may join over UDP when the server serves it. Off, the
    /// shell is told `--transport ws` and joins over the WebSocket a browser uses:
    /// for a network that drops UDP silently, where the shell would otherwise
    /// wait out its UDP attempt on every join.
    pub game_use_udp: bool,
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
            update_channel: "stable".into(),
            updater_endpoint: None,
            updater_pubkey: None,
            live_latency: true,
            latency_interval_ms: 1_000,
            latency_concurrency: 8,
            minimise_to_tray: false,
            download_limit_bps: 0,
            download_concurrency: crate::download::DEFAULT_CONCURRENT as u8,
            download_keep_history: true,
            require_signed_plugins: false,
            confirm_every_run: true,
            allow_config_editing: true,
            game_cache_keep: true,
            game_cache_limit_mb: 0,
            game_use_udp: true,
        }
    }
}

/// The floor and ceiling on [`AppSettings::latency_interval_ms`].
///
/// The floor is the load-bearing half. Every tick sends the whole visible set
/// of game servers a datagram from the user's address, so an interval of `0`
/// — a plausible typo, and what a slider dragged to its end would send if the
/// UI ever forgot its own bound — is an unintentional flood aimed at a third
/// party. 250ms is faster than a human reads a number and still leaves any
/// real server's round trip room to land between ticks.
///
/// The ceiling only stops "off" being expressed as a number; that is what
/// [`AppSettings::live_latency`] is for.
pub const LATENCY_INTERVAL_MS_MIN: u32 = 250;
pub const LATENCY_INTERVAL_MS_MAX: u32 = 300_000;

impl AppSettings {
    /// Clamp anything a hand-edited file could get wrong.
    ///
    /// The file is user-writable by design, so these are not paranoia about an
    /// attacker — they are about a typo turning into an unusable UI or a
    /// thousand concurrent sockets.
    fn sanitise(&mut self) {
        self.ui_scale = self.ui_scale.clamp(0.85, 1.4);
        self.latency_concurrency = self.latency_concurrency.clamp(1, 32);
        self.download_concurrency = self
            .download_concurrency
            .clamp(1, crate::download::MAX_CONCURRENT as u8);
        self.latency_interval_ms = self
            .latency_interval_ms
            .clamp(LATENCY_INTERVAL_MS_MIN, LATENCY_INTERVAL_MS_MAX);

        let theme_ok = matches!(self.theme.as_str(), "system" | "light" | "dark")
            || self
                .theme
                .strip_prefix("plugin:")
                .is_some_and(crate::plugins::manifest::is_valid_plugin_id);

        if !theme_ok {
            self.theme = "system".into();
        }

        // A terabyte is past anything a player means; a typo is not a reason
        // to hand the game an unbounded cache.
        self.game_cache_limit_mb = self.game_cache_limit_mb.min(1024 * 1024);

        if !crate::updater::CHANNELS.contains(&self.update_channel.as_str()) {
            self.update_channel = "stable".into();
        }

        /*
         * A hand-edited file is a way to set these without the command's
         * validation, so the same validation runs on read: a value the
         * command would refuse is dropped rather than trusted. Dropping falls
         * back to the compiled source, which is the safe direction.
         */
        self.updater_endpoint = self
            .updater_endpoint
            .take()
            .and_then(|e| crate::updater::validate_endpoint(&e).ok());
        self.updater_pubkey = self
            .updater_pubkey
            .take()
            .and_then(|k| crate::updater::validate_pubkey(&k).ok());

        /*
         * The jail roots are not re-validated here — that is `set_game_dir`'s
         * job and `patch` refuses to carry them at all — but they are RESPELT.
         *
         * These were stored straight from `std::fs::canonicalize`, which on
         * Windows returns `\\?\F:\SteamLibrary`. It names the right folder and
         * it is what every screen shows, so a settings file written before
         * `crate::canon` existed puts a verbatim prefix in front of every game
         * path in the Library. A directory is the same directory under both
         * spellings, so fixing it on read costs nothing and saves everybody
         * re-running the scan.
         */
        let respell = |dir: &mut String| {
            *dir = crate::canon::simplify(PathBuf::from(&*dir))
                .to_string_lossy()
                .into_owned();
        };

        self.game_dirs.values_mut().for_each(respell);

        if let Some(dir) = self.download_dir.as_mut() {
            respell(dir);
        }
    }
}

/// The settings that are jail roots rather than preferences.
///
/// Everything else in [`AppSettings`] describes how the app looks or behaves,
/// and the worst a wrong value does is annoy someone. These two decide where an
/// installer plugin is allowed to write, so they are the only fields with their
/// own setters and their own validation — see [`crate::anchor`].
///
/// Named with the wire (camelCase) spelling because that is what arrives in a
/// patch from the webview.
pub const JAIL_ROOT_FIELDS: &[&str] = &["gameDirs", "downloadDir"];

/// The settings that decide what the app's updater TRUSTS.
///
/// Refused by [`SettingsStore::patch`], and settable by NO command: a
/// public-key override chooses which signatures install over this program, and
/// the webview is assumed hostile. They are read only from a hand-edited
/// `settings.json` (validated on read, invalid values dropped) or come from the
/// build environment; an override in force is audited at Security level at
/// startup. Code that can write `settings.json` already owns the machine.
pub const UPDATER_SOURCE_FIELDS: &[&str] = &["updaterEndpoint", "updaterPubkey"];

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
    ///
    /// [`JAIL_ROOT_FIELDS`] are REFUSED here rather than merged. They are the
    /// anchors of the plugin jail, not preferences, and they go through
    /// [`Self::set_game_dir`] / [`Self::set_download_dir`], which run
    /// [`crate::anchor::validate_root`]. Refusing loudly rather than dropping
    /// the key silently: a caller that sends one has misunderstood which of
    /// these it is holding, and a settings write that appears to succeed and
    /// changes nothing is the worse failure.
    pub fn patch(&self, patch: serde_json::Value) -> AppResult<AppSettings> {
        let mut merged = serde_json::to_value(self.get())?;

        if let Some(over) = patch.as_object() {
            for field in JAIL_ROOT_FIELDS {
                if over.contains_key(*field) {
                    return Err(AppError::invalid(format!(
                        "`{field}` is a jail root and cannot be set through a settings patch."
                    )));
                }
            }

            for field in UPDATER_SOURCE_FIELDS {
                if over.contains_key(*field) {
                    return Err(AppError::invalid(format!(
                        "`{field}` decides what the updater trusts and cannot be set through a settings patch."
                    )));
                }
            }
        }

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

    /// Point a game's jail root at `dir`, or clear it with `None`.
    ///
    /// The path is validated and canonicalised by [`crate::anchor::validate_root`]
    /// before it is stored, so what lands in `settings.json` is the same path
    /// `plugins::jail` will resolve when it builds the jail.
    ///
    /// Returns the stored (canonical) path alongside the new settings, because
    /// the caller has to audit the value that was actually written rather than
    /// the one it was handed.
    pub fn set_game_dir(
        &self,
        app_id: &str,
        dir: Option<&str>,
        protected: &[PathBuf],
    ) -> AppResult<(AppSettings, Option<String>)> {
        if app_id.trim().is_empty() || !app_id.chars().all(|c| c.is_ascii_digit()) {
            return Err(AppError::invalid("That is not a game id."));
        }

        let mut next = self.get();

        let stored = match dir {
            Some(raw) => {
                let root = crate::anchor::validate_root(raw, protected)?;
                let text = root.to_string_lossy().into_owned();

                next.game_dirs.insert(app_id.to_string(), text.clone());

                Some(text)
            }
            None => {
                next.game_dirs.remove(app_id);

                None
            }
        };

        Ok((self.commit(next)?, stored))
    }

    /// Where installers put downloads before unpacking them, or `None` for the
    /// app's own cache. Same validation as a game root: it is a jail root
    /// too, and a plugin's `downloads` grant resolves beneath it.
    pub fn set_download_dir(
        &self,
        dir: Option<&str>,
        protected: &[PathBuf],
    ) -> AppResult<(AppSettings, Option<String>)> {
        let mut next = self.get();

        let stored = match dir {
            Some(raw) => {
                let root = crate::anchor::validate_root(raw, protected)?;

                Some(root.to_string_lossy().into_owned())
            }
            None => None,
        };

        next.download_dir = stored.clone();

        Ok((self.commit(next)?, stored))
    }

    /// Sanitise, persist and publish. Shared by the setters above so a new one
    /// cannot forget the clamp or the in-memory update.
    fn commit(&self, mut next: AppSettings) -> AppResult<AppSettings> {
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
        /*
         * The updater overrides survive a reset: an admin put them in the file
         * by hand, and no command can put them back.
         */
        let kept = self.get();
        let fresh = AppSettings {
            updater_endpoint: kept.updater_endpoint,
            updater_pubkey: kept.updater_pubkey,
            ..AppSettings::default()
        };
        self.write(&fresh)?;

        if let Ok(mut cur) = self.current.write() {
            *cur = fresh.clone();
        }

        Ok(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_join_over_udp_unless_the_player_turns_it_off() {
        assert!(AppSettings::default().game_use_udp);

        let parsed: AppSettings = serde_json::from_str("{}").expect("empty settings parse");

        assert!(parsed.game_use_udp);
    }

    struct TempStore(PathBuf, SettingsStore);

    impl TempStore {
        fn new(tag: &str) -> Self {
            let base =
                std::env::temp_dir().join(format!("tmc-settings-{tag}-{}", std::process::id()));

            std::fs::create_dir_all(&base).expect("temp dir");

            let store = SettingsStore::load(base.join("settings.json"));

            Self(base, store)
        }
    }

    impl Drop for TempStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The whole point of `JAIL_ROOT_FIELDS`. A patch is the surface the
    /// webview reaches directly, so this is the gate that has to hold.
    #[test]
    fn a_patch_cannot_set_a_jail_root() {
        let store = TempStore::new("roots");

        for field in JAIL_ROOT_FIELDS {
            let patch = serde_json::json!({ *field: "/tmp" });

            assert!(
                store.1.patch(patch).is_err(),
                "`{field}` must not be settable through a patch"
            );
        }
    }

    /// A patch is the surface the webview reaches directly; what the updater
    /// trusts is not settable from there at all.
    #[test]
    fn a_patch_cannot_change_what_the_updater_trusts() {
        let store = TempStore::new("updater-patch");

        for field in UPDATER_SOURCE_FIELDS {
            let patch = serde_json::json!({ *field: "https://evil.example/latest.json" });

            assert!(
                store.1.patch(patch).is_err(),
                "`{field}` must not be patchable"
            );
        }

        // The channel is a preference and IS patchable, clamped to the two.
        let next = store
            .1
            .patch(serde_json::json!({ "updateChannel": "beta" }))
            .expect("channel");
        assert_eq!(next.update_channel, "beta");

        let next = store
            .1
            .patch(serde_json::json!({ "updateChannel": "nightly" }))
            .expect("channel");
        assert_eq!(next.update_channel, "stable");
    }

    #[test]
    fn a_hand_edited_updater_source_is_validated_on_read_and_survives_reset() {
        let store = TempStore::new("updater-source");
        let file = store.0.join("settings.json");
        let key = crate::updater::TEST_PUBKEY;

        let write = |endpoint: &str, pubkey: &str| {
            std::fs::write(
                &file,
                serde_json::json!({ "updaterEndpoint": endpoint, "updaterPubkey": pubkey })
                    .to_string(),
            )
            .expect("write");
        };

        write("http://evil.example/latest.json", "not a key");
        let refused = SettingsStore::load(file.clone()).get();
        assert_eq!(refused.updater_endpoint, None, "invalid is dropped");
        assert_eq!(refused.updater_pubkey, None);

        write("https://cdn.example/latest.json", key);
        let loaded = SettingsStore::load(file.clone());
        assert_eq!(
            loaded.get().updater_endpoint.as_deref(),
            Some("https://cdn.example/latest.json")
        );
        assert_eq!(loaded.get().updater_pubkey.as_deref(), Some(key));

        let fresh = loaded.reset().expect("reset");
        assert_eq!(fresh.updater_pubkey.as_deref(), Some(key), "reset keeps it");
        assert!(SettingsStore::load(file).get().updater_endpoint.is_some());
    }

    /// The refusal must be scoped to those two fields and nothing else, or
    /// every ordinary settings screen breaks.
    #[test]
    fn an_ordinary_patch_still_applies() {
        let store = TempStore::new("ordinary");

        let next = store
            .1
            .patch(serde_json::json!({ "compactCards": true, "uiScale": 1.2 }))
            .expect("applied");

        assert!(next.compact_cards);
        assert!((next.ui_scale - 1.2).abs() < f32::EPSILON);
    }

    /// A refused patch must leave the stored value alone — a partial write
    /// would be worse than the rejection it is trying to be.
    #[test]
    fn a_refused_patch_changes_nothing_at_all() {
        let store = TempStore::new("atomic");

        store
            .1
            .patch(serde_json::json!({ "compactCards": true }))
            .expect("applied");

        let refused = store
            .1
            .patch(serde_json::json!({ "compactCards": false, "downloadDir": "/tmp" }));

        assert!(refused.is_err());
        assert!(
            store.1.get().compact_cards,
            "the sibling field was written anyway"
        );
    }

    /// The refresh interval is the one preference here that points outward: it
    /// decides how often somebody else's game server is sent a packet. A patch
    /// from the webview must not be able to take it below the floor.
    #[test]
    fn the_refresh_interval_is_clamped_from_both_ends() {
        let store = TempStore::new("interval");

        let fast = store
            .1
            .patch(serde_json::json!({ "latencyIntervalMs": 0 }))
            .expect("applied");

        assert_eq!(fast.latency_interval_ms, LATENCY_INTERVAL_MS_MIN);

        let slow = store
            .1
            .patch(serde_json::json!({ "latencyIntervalMs": u32::MAX }))
            .expect("applied");

        assert_eq!(slow.latency_interval_ms, LATENCY_INTERVAL_MS_MAX);
    }

    #[test]
    fn a_game_dir_must_be_keyed_by_a_numeric_app_id() {
        let store = TempStore::new("appid");
        let dir = std::env::temp_dir();
        let dir = dir.to_str().expect("utf8");

        assert!(store.1.set_game_dir("../etc", Some(dir), &[]).is_err());
        assert!(store.1.set_game_dir("", Some(dir), &[]).is_err());
    }

    #[test]
    fn clearing_a_game_dir_needs_no_validation() {
        let store = TempStore::new("clear");

        let (next, stored) = store.1.set_game_dir("42", None, &[]).expect("cleared");

        assert!(stored.is_none());
        assert!(!next.game_dirs.contains_key("42"));
    }
}
