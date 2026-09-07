use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use tauri::AppHandle;

use tmc_core::api::ApiClient;
use tmc_core::auth::AuthState;
use tmc_core::crypto::LocalCipher;
use tmc_core::download::DownloadManager;
use tmc_core::error::{AppError, AppResult};
use tmc_core::launch::{plan as build_launch_plan, LaunchContext, LaunchOptions, LaunchPlan};
use tmc_core::library::LibraryDb;
use tmc_core::local::vault::PathVault;
use tmc_core::logging::Audit;
use tmc_core::net::LatencyStore;
use tmc_core::plugins::apps::AppPlugins;
use tmc_core::plugins::registry::Registry;
use tmc_core::plugins::{jail_for, JailRoots};
use tmc_core::rcon::RconPool;
use tmc_core::secure::SecureStore;
use tmc_core::session::{Session, SessionKind, Sessions};
use tmc_core::settings::{AppSettings, SettingsStore};

use crate::paths::AppPaths;

/// The three directories a sandbox operation resolves against.
pub struct SandboxDirs {
    /// `<data>/staging` — per sandbox, per mod.
    pub staging: PathBuf,
    /// `<data>/backups` — what a deploy displaced.
    pub backups: PathBuf,
    /// `<data>/local-mods` — the device's imported mods, shared across
    /// sandboxes. See `tmc_core::local`.
    pub local: PathBuf,
}

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

    /// The device's library: which subscriptions are here, what is on disk,
    /// every sandbox, the deployment ledger and the download queue.
    pub library: Arc<LibraryDb>,

    /// The download queue. One per process — a second would mean two schedulers
    /// racing for the same concurrency budget and two writers per `.part` file.
    pub downloads: DownloadManager,

    /// Open RCON sessions, pooled so a console is a conversation rather than a
    /// reconnect per line.
    pub rcon: RconPool,

    /// Slug → TMC app id, learned from the catalogue.
    ///
    /// The device's plugin folders are named after a game's URL slug and
    /// everything it STORES is keyed by the numeric id, and until this existed
    /// the only translation was "find a sandbox or a subscription that already
    /// has both". That works for a machine somebody has already set up and
    /// fails for the one case the folder scan exists to serve: a fresh install
    /// with an empty library, where every scan result was found and then could
    /// not be applied.
    ///
    /// Cached because it is asked once per candidate in a batch of a dozen, and
    /// the mapping changes when a game is added to the catalogue.
    app_ids: RwLock<std::collections::BTreeMap<String, i64>>,

    /// One playtime flush at a time.
    ///
    /// It reads the unreported rows, sends them, and only then marks them —
    /// so two overlapping runs read the same rows and each report the same
    /// seconds. The library's sync loop fires it WITHOUT awaiting it (the
    /// report must not hold up a sync), so its own re-entrancy guard has
    /// already been released by the time a second pass starts.
    flushing: Arc<std::sync::atomic::AtomicBool>,

    /// One filesystem scan at a time, and a way to stop it.
    ///
    /// Two flags rather than one because they answer different questions and
    /// are set by different sides: `scan_running` is owned by the scan and says
    /// whether a second may start, while `scan_cancel` is set by the UI and
    /// read by the walk. Folding them into one would mean cancelling by
    /// clearing "running", which is indistinguishable from the scan finishing.
    scan_running: Arc<std::sync::atomic::AtomicBool>,
    scan_cancel: Arc<std::sync::atomic::AtomicBool>,

    /// Games this process has started, live and recent.
    ///
    /// An `Arc` because the supervisor thread that waits on each child holds
    /// one — see [`tmc_core::session::Sessions::spawn`]. One registry per
    /// process, like the download queue and for the same reason: two would each
    /// know about half the running games, and the half a deploy checked would
    /// be the wrong half.
    pub sessions: Arc<Sessions>,

    /// The key that encrypts RCON passwords.
    ///
    /// Created LAZILY, on first use. A user who never adds a server never has a
    /// key — so there is nothing to steal, nothing to back up and nothing to
    /// migrate — and the credential-store round trip does not happen on a
    /// launch that will not need it.
    cipher: std::sync::OnceLock<LocalCipher>,

    /// Paths this process found, addressable from the webview by token.
    ///
    /// Filled by the window's drag-drop handler and by the two import scans.
    /// See [`PathVault`] for why the indirection exists at all.
    pub vault: PathVault,

    /// The most recent drop on the window, waiting to be collected.
    ///
    /// Held as well as emitted, because an event fires whether or not anything
    /// is listening: a drop that lands during a route change would otherwise be
    /// silently lost, which reads as drag and drop not working rather than as a
    /// race. One slot, not a queue — a second drop before the first is handled
    /// replaces it, which is what somebody dropping again after nothing seemed
    /// to happen actually means.
    drop: RwLock<Option<crate::commands::import::DropBatch>>,

    /// App-scoped install and launch rules (`plugins/app/<slug>/…`).
    ///
    /// An `Arc` behind an `RwLock` so a reload swaps the whole set in one write
    /// while every reader keeps the snapshot it started with. Handing out a
    /// GUARD instead would mean an install holding a read lock for the length
    /// of a download, and a reload blocking behind it.
    app_plugins: RwLock<Arc<AppPlugins>>,
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

        /*
         * The download manager gets its OWN http client rather than the API's.
         * The API client attaches a bearer to everything it sends, and a mod
         * file comes from a CDN that has no business seeing one — a redirect to
         * a third-party mirror would hand somebody's access token to a host we
         * do not control.
         */
        let file_http = tmc_core::download::default_client(&version)?;

        let plugins = Registry::load(paths.registry_file(), paths.plugins.clone());

        /*
         * The registry holds no settings store, so the one setting that gates
         * what may RUN is pushed into it — here at startup and again on every
         * `settings_patch`. Reading it the other way round would make the gate
         * depend on which of the two was constructed first.
         */
        plugins.set_require_signed(current.require_signed_plugins);

        let library = Arc::new(LibraryDb::open(paths.library_file())?);

        /*
         * Anything left mid-flight by a crash is reset here. A row stuck at
         * `installing` is not a state anything can act on — the process that
         * owned it is gone — and leaving it means the item never installs again
         * while the UI shows a spinner forever.
         */
        match library.reset_transient_states() {
            Ok(0) => {}
            Ok(n) => tracing::warn!("reset {n} interrupted library operation(s)"),
            Err(e) => tracing::warn!("could not reset library states: {}", e.detail()),
        }

        /*
         * A download left `running` by a crash comes back `queued`. Same
         * reasoning as the library's transient states: the process that owned
         * it is gone, so nothing will ever move it and the UI would show a
         * stalled bar forever.
         */
        match library.download_reset_running() {
            Ok(0) => {}
            Ok(n) => tracing::warn!("requeued {n} interrupted download(s)"),
            Err(e) => tracing::warn!("could not requeue downloads: {}", e.detail()),
        }

        // `resolve`, not `load`: the rules the app SHIPS are compiled into the
        // binary and the user's directory is an overlay on them. `load` reads
        // only the overlay, which on a fresh install is empty — and an empty
        // rule set is not an error anywhere, it is just an app that detects no
        // games and launches nothing.
        let app_plugins = AppPlugins::resolve(&paths.plugins);

        for (source, error) in app_plugins.errors() {
            tracing::warn!("app plugin {source} did not load: {error}");
        }

        let downloads = DownloadManager::new(file_http, Arc::clone(&audit));
        let rcon = RconPool::new(Arc::clone(&audit));

        let sessions = Arc::new(Sessions::new(paths.logs.join("sessions")));

        /*
         * A finished session is WRITTEN, not reported.
         *
         * The hook runs on the supervisor thread the moment a game exits, and
         * that thread has no runtime, no network and nowhere to put a failure.
         * More importantly a game is very often played offline — a laptop on a
         * train is the case this feature is for — and playtime that only counts
         * when the network happened to be up is playtime that silently goes
         * missing. So the row lands in the database marked unreported and
         * `sessions_flush` sends it whenever the device next has an API to talk
         * to.
         */
        {
            let db = Arc::clone(&library);
            let audit = Arc::clone(&audit);

            sessions.on_end(Arc::new(move |session: &Session| {
                let row = session_row(session);

                if let Err(err) = db.session_record(&row) {
                    tracing::warn!("could not record play session: {}", err.detail());
                }

                tmc_core::audit!(
                    audit,
                    Info,
                    App,
                    "session.end",
                    format!(
                        "{} ran for {}s",
                        session.label,
                        session.seconds(tmc_core::session::now_ms()).unwrap_or(0)
                    )
                );
            }));
        }

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
            downloads,
            rcon,
            sessions,
            app_ids: RwLock::new(std::collections::BTreeMap::new()),
            flushing: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            scan_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            scan_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cipher: std::sync::OnceLock::new(),
            vault: PathVault::new(),
            drop: RwLock::new(None),
            library,
            app_plugins: RwLock::new(Arc::new(app_plugins)),
        })
    }

    /// Record what was just dropped on the window.
    pub fn set_drop(&self, batch: crate::commands::import::DropBatch) {
        if let Ok(mut slot) = self.drop.write() {
            *slot = Some(batch);
        }
    }

    /// Collect it, once.
    ///
    /// Taken rather than read: a batch that has been handed to the UI has been
    /// handled, and leaving it in place means a later remount re-opens an
    /// import dialog for files the user already dealt with.
    pub fn take_drop(&self) -> Option<crate::commands::import::DropBatch> {
        self.drop.write().ok()?.take()
    }

    /// The app id for a game's plugin slug, from the cache.
    pub fn cached_app_id(&self, slug: &str) -> Option<i64> {
        self.app_ids
            .read()
            .ok()?
            .get(&slug.to_ascii_lowercase())
            .copied()
    }

    /// Learn a batch of slug → id mappings.
    pub fn remember_app_ids(&self, pairs: impl IntoIterator<Item = (String, i64)>) {
        let Ok(mut map) = self.app_ids.write() else {
            return;
        };

        for (slug, id) in pairs {
            map.insert(slug.to_ascii_lowercase(), id);
        }
    }

    /// The plugin slug for a TMC app id, from anything on this device.
    ///
    /// The inverse of [`Self::cached_app_id`], and it matters for the same
    /// reason: a sandbox with no slug has no game rules at all — no preset, no
    /// strategy validation, no install rule and no launch rule. It is created
    /// successfully and then does nothing, which is the worst way to fail.
    ///
    /// Local sources first because they are free and authoritative for a game
    /// this device already has; the catalogue cache is the fallback that covers
    /// a machine with an empty library.
    pub fn app_slug_for(&self, app_id: i64) -> Option<String> {
        if let Ok(sandboxes) = self.library.sandbox_list(None) {
            if let Some(found) = sandboxes
                .iter()
                .find(|s| s.app_id == app_id && s.app_slug.is_some())
            {
                return found.app_slug.clone();
            }
        }

        if let Ok(rows) = self.library.list() {
            if let Some(slug) = rows
                .into_iter()
                .find(|e| e.app_id == Some(app_id) && e.app_slug.is_some())
                .and_then(|e| e.app_slug)
            {
                return Some(slug);
            }
        }

        self.app_ids
            .read()
            .ok()?
            .iter()
            .find(|(_, id)| **id == app_id)
            .map(|(slug, _)| slug.clone())
    }

    /// Claim the playtime-flush slot. False when one is already running.
    ///
    /// `compare_exchange` for the reason `scan_begin` uses one: a read followed
    /// by a write is not a claim, and two syncs a moment apart is exactly the
    /// pattern this loses.
    pub fn flush_begin(&self) -> bool {
        self.flushing
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
    }

    pub fn flush_end(&self) {
        self.flushing
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    /// Claim the scan slot. False when one is already running.
    ///
    /// `compare_exchange` rather than a read followed by a write: two Scan
    /// buttons pressed a frame apart on a fast machine both pass a read-then-
    /// write and start two walks, which is the exact race this is here to lose.
    pub fn scan_begin(&self) -> bool {
        self.scan_running
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
    }

    /// Release the scan slot. Called on every path, including the failing one.
    pub fn scan_end(&self) {
        self.scan_running
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn scan_running(&self) -> bool {
        self.scan_running.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The flag the walk checks between directories.
    pub fn scan_cancel(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.scan_cancel)
    }

    /// The key that encrypts RCON passwords, created on first use.
    ///
    /// Every failure is the same one — the credential store is unreachable, or
    /// the stored key is corrupt — and it is worth surfacing rather than
    /// silently minting a replacement, which would turn "the entry is broken"
    /// into "every saved password is wrong".
    pub fn cipher(&self) -> AppResult<&LocalCipher> {
        if let Some(cipher) = self.cipher.get() {
            return Ok(cipher);
        }

        let created = LocalCipher::load_or_create(&self.secure)?;

        // A race here means two keys were generated and one is discarded — but
        // `load_or_create` writes before returning, so both loaded the SAME key
        // and the loser is identical to the winner.
        let _ = self.cipher.set(created);

        self.cipher
            .get()
            .ok_or_else(|| AppError::internal("cipher was not initialised"))
    }

    /// The three directories every sandbox operation resolves against.
    ///
    /// Held together rather than passed as three arguments, because they are
    /// three answers to one question — "where does this device keep mod files?"
    /// — and a call site that assembled two of them and forgot the third would
    /// compile perfectly and deploy from the wrong place.
    pub fn sandbox_dirs(&self) -> SandboxDirs {
        SandboxDirs {
            staging: self.paths.staging_dir(),
            backups: self.paths.backup_dir(),
            local: self.paths.local_mods_dir(),
        }
    }

    /// Everything a sandbox operation needs, assembled from this state.
    pub fn sandbox_ctx<'a>(
        &'a self,
        plugins: &'a tmc_core::plugins::apps::AppPlugins,
        settings: &'a AppSettings,
        roots: &'a tmc_core::plugins::JailRoots,
        dirs: &'a SandboxDirs,
    ) -> tmc_core::library::deploy::SandboxCtx<'a> {
        tmc_core::library::deploy::SandboxCtx {
            plugins,
            roots,
            settings,
            http: self.api.raw(),
            audit: &self.audit,
            downloads: Some(&self.downloads),
            staging_root: &dirs.staging,
            backup_root: &dirs.backups,
            local_root: &dirs.local,
        }
    }

    /// Where a file the USER asked for by name goes.
    ///
    /// Their configured download folder when they have set one, and otherwise
    /// the app's own cache — which is swept on launch, and is the right default
    /// precisely because a file nobody chose a home for should not accumulate
    /// somewhere they will never think to look.
    ///
    /// Not `settings.download_dir` read directly at the call site: that field
    /// is a jail anchor with its own validating command, and having one place
    /// answer "where does a download go" keeps the fallback from being
    /// reinvented differently the second time somebody needs it.
    pub fn download_target_dir(&self) -> std::path::PathBuf {
        self.settings
            .get()
            .download_dir
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| self.paths.cache.join("downloads"))
    }

    /// The platform directories game detection reads.
    ///
    /// From Tauri's resolver, never from an environment variable — the same
    /// rule every other path in this app follows, and the reason `tmc_core::
    /// detect` takes them as an argument instead of finding them itself.
    pub fn detect_roots(&self, app: &AppHandle) -> tmc_core::detect::DetectRoots {
        use tauri::Manager;

        let resolver = app.path();

        #[allow(unused_mut)]
        let mut program_files: Vec<PathBuf> = Vec::new();
        #[allow(unused_mut)]
        let mut drives: Vec<PathBuf> = Vec::new();

        #[cfg(windows)]
        {
            for letter in b'A'..=b'Z' {
                let root = PathBuf::from(format!("{}:\\", letter as char));

                if !root.is_dir() {
                    continue;
                }

                for rel in ["Program Files", "Program Files (x86)"] {
                    let candidate = root.join(rel);

                    if candidate.is_dir() {
                        program_files.push(candidate);
                    }
                }

                drives.push(root);
            }
        }

        tmc_core::detect::DetectRoots {
            home: resolver.home_dir().ok(),
            config: resolver.config_dir().ok(),
            local_data: resolver.local_data_dir().ok(),
            program_data: program_data_dir(),
            program_files,
            drives,
        }
    }

    /// The directories a plugin jail may be anchored to.
    ///
    /// Resolved here rather than in `tmc-core` because the answer is
    /// platform-specific and Tauri's resolver is the only thing that knows it.
    /// The core decides what a plugin may *do* with them.
    pub fn jail_roots(&self) -> JailRoots {
        JailRoots {
            data: self.paths.data.clone(),
            cache: self.paths.cache.clone(),
        }
    }

    /// A snapshot of the app-scoped rules.
    ///
    /// Cloning the `Arc`, not the rules. A caller keeps the set it started with
    /// for as long as it holds the handle, which is exactly what an install
    /// that takes two minutes wants — a rule reload mid-download must not
    /// change what is being installed.
    pub fn app_plugins(&self) -> Arc<AppPlugins> {
        self.app_plugins
            .read()
            .map(|g| Arc::clone(&g))
            .unwrap_or_else(|e| Arc::clone(&e.into_inner()))
    }

    /// Re-read `plugins/app/`. Cheap, and the only way to pick up a rule the
    /// user just dropped in without restarting.
    pub fn reload_app_plugins(&self) {
        let fresh = Arc::new(AppPlugins::resolve(&self.paths.plugins));

        if let Ok(mut slot) = self.app_plugins.write() {
            *slot = fresh;
        }
    }

    pub fn settings_snapshot(&self) -> AppSettings {
        self.settings.get()
    }

    // ------------------------------------------------------------- Installs

    /// One mirrored install's payload, parsed.
    /// The account's own record of one install, as it was last synced.
    ///
    /// `pub(crate)` rather than private because the launcher needs the same
    /// three facts — the game, its slug and the install's name — to label a
    /// play session, and re-deriving them from the library's item rows gets a
    /// different answer: a library row is a subscribed MOD, and an install with
    /// no mods in it has no row at all.
    pub(crate) fn install_payload(&self, id: i64) -> Option<serde_json::Value> {
        self.library
            .install_payloads()
            .ok()?
            .into_iter()
            .filter_map(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .find(|v| v.get("id").and_then(serde_json::Value::as_i64) == Some(id))
    }

    /// The default install for an app, if one has been mirrored.
    pub fn default_install_for(&self, app_id: i64) -> Option<i64> {
        self.library
            .install_payloads()
            .ok()?
            .into_iter()
            .filter_map(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .find(|v| {
                v.get("appId").and_then(serde_json::Value::as_i64) == Some(app_id)
                    && v.get("isDefault")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
            })
            .and_then(|v| v.get("id").and_then(serde_json::Value::as_i64))
    }

    /// The four facts an install contributes to an install run: its directory
    /// on THIS machine, its loader, its game version and its name.
    pub fn install_facts(
        &self,
        id: i64,
    ) -> (
        Option<PathBuf>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        let dir = self
            .library
            .install_local_dir(id)
            .ok()
            .flatten()
            .map(PathBuf::from);

        let Some(payload) = self.install_payload(id) else {
            return (dir, None, None, None);
        };

        let string = |key: &str| {
            payload
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };

        (dir, string("loader"), string("gameVersion"), string("name"))
    }

    // --------------------------------------------------------------- Launch

    /// Resolve what launching an install would run.
    ///
    /// Everything is looked up here rather than accepted from the caller: the
    /// rule, the game directory, the options. A frontend that could supply any
    /// of them would make every check in `tmc_core::launch` advisory.
    pub fn launch_plan(&self, install_id: i64) -> AppResult<LaunchPlan> {
        let payload = self
            .install_payload(install_id)
            .ok_or_else(|| AppError::invalid("That install is not on this device yet."))?;

        let app_id = payload
            .get("appId")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| AppError::invalid("That install names no app."))?;

        let slug = payload
            .get("app")
            .and_then(|a| a.get("slug"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| AppError::invalid("That install's app has no slug."))?;

        let (dir, loader, game_version, name) = self.install_facts(install_id);

        let plugins = self.app_plugins();

        let rule = plugins
            .launch_for(&slug, loader.as_deref())
            .ok_or_else(|| AppError::invalid("This app has no launch rule."))?;

        /*
         * The install's own directory becomes the `gameDir` root, exactly as it
         * does for an install run — the same mechanism rather than a second
         * one, so a profile can never end up with laxer path checks than the
         * main install.
         */
        let mut settings = self.settings.get();

        if let Some(dir) = &dir {
            settings
                .game_dirs
                .insert(app_id.to_string(), dir.display().to_string());
        }

        let manifest = rule.as_manifest();
        let roots = self.jail_roots();

        let jail = jail_for(&manifest, &roots, &settings, Some(app_id))?;

        let options: LaunchOptions = payload
            .get("options")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let ctx = LaunchContext {
            install_name: name,
            game_version,
            loader,
            install_dir: dir,
            game_dir: jail
                .root_path(tmc_core::plugins::manifest::FsRoot::GameDir)
                .map(std::path::Path::to_path_buf),
            /*
             * An INSTALL never carries one. A virtual filesystem belongs to a
             * sandbox — it is that sandbox's merge tree — and launching the
             * install is launching the game as it is on disk, which for every
             * strategy but this one is the same thing.
             */
            vfs: None,
        };

        let mut plan = build_launch_plan(rule, &jail, &options, &ctx)?;

        /*
         * The install's own `launchArgs` and `launchEnv` are appended AFTER the
         * rule's. The rule knows how to start the game; the user's extra flags
         * are theirs, and a game's argument parser takes the last occurrence of
         * a repeated flag — so appending is what makes an override actually
         * override.
         */
        if let Some(extra) = payload
            .get("launchArgs")
            .and_then(serde_json::Value::as_array)
        {
            for value in extra {
                let Some(arg) = value.as_str() else { continue };

                // Same refusal the rule's own arguments get. A user-typed
                // argument is not more trusted than a plugin-supplied one.
                if arg.contains('\0') || arg.contains('\n') || arg.contains('\r') {
                    return Err(AppError::jail(
                        "A launch argument contains a control character.",
                    ));
                }

                plan.args.push(arg.to_string());
            }
        }

        if let Some(env) = payload
            .get("launchEnv")
            .and_then(serde_json::Value::as_object)
        {
            for (key, value) in env {
                let Some(value) = value.as_str() else {
                    continue;
                };

                if key.contains('\0') || value.contains('\0') {
                    return Err(AppError::jail(
                        "A launch environment value contains a NUL byte.",
                    ));
                }

                plan.env.insert(key.clone(), value.to_string());
            }
        }

        if plan.args.len() > 128 {
            return Err(AppError::invalid(
                "That launch produces too many arguments.",
            ));
        }

        Ok(plan)
    }
}

/// `%ProgramData%`, which Tauri's resolver does not expose.
///
/// Derived from the system drive rather than read from the environment, for the
/// same reason `api_base` is not read from one in a release build: the
/// environment of the process that launched the app is not a trust boundary,
/// and a shortcut's "Start in" can set anything. It is only ever used to LOOK
/// for a launcher's manifests, never to write.
fn program_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        for letter in b'C'..=b'Z' {
            let candidate = PathBuf::from(format!("{}:\\ProgramData", letter as char));

            if candidate.is_dir() {
                return Some(candidate);
            }
        }

        None
    }

    #[cfg(not(windows))]
    {
        None
    }
}

/// Turn a live session into the row the database stores.
///
/// The `seconds` conversion is the whole reason this is a function rather than
/// a `From` impl on the core type: `Session::seconds` answers `None` for a
/// launch whose duration is unknowable (a `steam://` handoff), and the column
/// stores `0` for exactly that case. Collapsing `None` to `0` is right HERE and
/// wrong at every other call site, where the difference between "did not play"
/// and "cannot say" is the thing being displayed.
fn session_row(session: &Session) -> tmc_core::library::SessionRow {
    let kind = match session.kind {
        SessionKind::Process => "process",
        SessionKind::Handoff => "handoff",
        SessionKind::Web => "web",
    };

    tmc_core::library::SessionRow {
        // Assigned by SQLite on insert.
        id: 0,
        kind: kind.to_string(),
        app_id: session.app_id,
        app_slug: session.app_slug.clone(),
        label: session.label.clone(),
        sandbox_id: session.sandbox_id,
        install_id: session.install_id,
        started_ms: session.started_ms,
        ended_ms: session.ended_ms,
        seconds: session
            .seconds(session.ended_ms.unwrap_or_else(tmc_core::session::now_ms))
            .unwrap_or(0),
        exit_code: session.exit_code,
        stopped_by_user: session.stopped_by_user,
    }
}
