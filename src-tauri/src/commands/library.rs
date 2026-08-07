//! The library's IPC surface: sync, install, uninstall, launch.
//!
//! The two rules from `commands/mod.rs` hold here as everywhere else — no
//! command returns a credential, and no command takes a filesystem path from
//! the webview. The one exception is `library_set_install_dir`, which takes the
//! path the user picked in a NATIVE folder dialog; it is stored, never
//! executed, and everything that later resolves against it goes through the
//! sandbox.
//!
//! A third rule is specific to this file: **nothing here decides on its own to
//! write to a game folder.** Every install runs because a subscription said so
//! and the user's own `autoUpdate` setting allowed it, and every one of them is
//! in the audit log before and after.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use tmc_core::audit;
use tmc_core::error::{AppError, AppResult};
use tmc_core::launch::{describe, LaunchPlan};
use tmc_core::library::db::LibraryEntry;
use tmc_core::library::install::{install_one, preflight, rule_for, uninstall_one, InstallCtx};
use tmc_core::library::{sync_installs, sync_once, InstallOutcome, SyncReport, FULL_SYNC_EVERY};
use tmc_core::plugins::apps::{AppPluginKind, AppPlugins};
use tmc_core::plugins::SandboxRoots;
use tmc_core::settings::AppSettings;

use crate::state::AppState;

/// How many sync passes have run this session.
///
/// Drives the "every tenth pass is a full one" rule. A process-lifetime counter
/// rather than a persisted one on purpose: a launch is ALWAYS a full sync (the
/// counter starts at zero), which is exactly when reconciling removals matters
/// most — the device may have been off while the user unsubscribed on their
/// phone.
static SYNC_PASSES: AtomicU32 = AtomicU32::new(0);

/// One row as the UI renders it: the server's fields, this device's, and the
/// two derived answers every screen wants.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRow {
    #[serde(flatten)]
    pub entry: LibraryEntry,
    /// Is a newer release available than the one on disk?
    pub update_available: bool,
    /// Is there a rule that could install this on this machine?
    pub has_rule: bool,
    /// A grouping rather than something with files — see
    /// `LibraryEntry::is_container`. A collection's MEMBERS are what install.
    pub is_container: bool,
}

fn row(entry: LibraryEntry, state: &AppState) -> LibraryRow {
    let plugins = state.app_plugins();

    LibraryRow {
        update_available: entry.update_available(),
        has_rule: rule_for(&entry, &plugins, None).is_some(),
        is_container: entry.is_container(),
        entry,
    }
}

/// The whole library, newest activity first.
#[tauri::command]
pub fn library_list(state: State<'_, AppState>) -> AppResult<Vec<LibraryRow>> {
    let mut rows: Vec<LibraryRow> = state
        .library
        .list()?
        .into_iter()
        .map(|e| row(e, &state))
        .collect();

    rows.sort_by(|a, b| {
        a.entry
            .name
            .to_lowercase()
            .cmp(&b.entry.name.to_lowercase())
    });

    Ok(rows)
}

/// Run one sync pass and act on its plan.
///
/// `force_full` is what the "Sync now" button sets. Everything else takes a
/// delta, with every `FULL_SYNC_EVERY`-th pass promoted to a full one so
/// deletions are reconciled — see `library::sync`.
#[tauri::command]
pub async fn library_sync(
    state: State<'_, AppState>,
    force_full: Option<bool>,
) -> AppResult<SyncReport> {
    let pass = SYNC_PASSES.fetch_add(1, Ordering::Relaxed);

    let full = force_full.unwrap_or(false) || pass % FULL_SYNC_EVERY == 0;

    // The installs mirror first: an install's directory decides WHERE the
    // subscriptions below land, and a device that has just learnt about a new
    // profile should use it on this pass rather than the next.
    if let Err(err) = sync_installs(&state.api, &state.library).await {
        tracing::warn!("install sync failed: {}", err.detail());
    }

    let report = sync_once(&state.api, &state.library, full).await?;

    audit!(
        state.audit,
        Info,
        Install,
        "library.sync",
        format!(
            "{} received, {} new, {} updated, {} removed",
            report.received, report.added, report.updated, report.removed
        )
    );

    /*
     * The plan is executed here rather than returned to the webview to act on.
     * A frontend that decides what to install is a frontend an injected script
     * can talk into installing something — and the whole architecture rests on
     * the webview being unable to reach the filesystem.
     */
    for id in &report.to_uninstall {
        if let Ok(Some(entry)) = state.library.get(id) {
            /*
             * `forget` deletes the ROW as well as the files, and is right only
             * for something the server no longer lists at all. A PAUSED
             * subscription is also uninstalled, and keeps its row — the user
             * still has it and simply asked for it not to be applied.
             */
            let forget = report.to_forget.contains(id);

            let holder = resolve_ctx(&state, &entry);

            let _ = uninstall_one(&state.library, &entry, &holder.ctx(&state), forget).await;
        }
    }

    for id in &report.to_install {
        if let Ok(Some(entry)) = state.library.get(id) {
            let holder = resolve_ctx(&state, &entry);

            let _ = install_one(&state.library, &entry, &holder.ctx(&state)).await;
        }
    }

    Ok(report)
}

/// Owns the pieces an [`InstallCtx`] borrows.
///
/// `InstallCtx` holds references — which is right for the core, where every
/// caller already has the values on its stack. Here they come out of an
/// `RwLock`, a settings store and a path resolver, so something has to keep
/// them alive for the length of the run. This is that something, and the
/// two-step (`resolve` then `ctx`) is what makes the borrow checker agree.
struct CtxHolder {
    plugins: Arc<AppPlugins>,
    roots: SandboxRoots,
    settings: AppSettings,
    install_id: Option<i64>,
    install_dir: Option<PathBuf>,
    loader: Option<String>,
    game_version: Option<String>,
    install_name: Option<String>,
}

impl CtxHolder {
    fn ctx<'a>(&'a self, state: &'a AppState) -> InstallCtx<'a> {
        InstallCtx {
            plugins: &self.plugins,
            roots: &self.roots,
            settings: &self.settings,
            http: state.api.raw(),
            audit: &state.audit,
            install_id: self.install_id,
            install_dir: self.install_dir.clone(),
            loader: self.loader.clone(),
            game_version: self.game_version.clone(),
            install_name: self.install_name.clone(),
        }
    }
}

/// Resolve which sandbox (profile) an entry belongs to, and that profile's
/// local directory.
///
/// An entry with no profile assignment falls back to the app's DEFAULT install
/// for its game, and then to the plain `game_dirs` entry — which is the "main
/// install" every user starts with and the only one most will ever have.
fn resolve_ctx(state: &AppState, entry: &LibraryEntry) -> CtxHolder {
    let profile = entry.installed_install_id.or_else(|| {
        entry
            .app_id
            .and_then(|app_id| state.default_install_for(app_id))
    });

    let (install_dir, loader, game_version, install_name) = match profile {
        Some(id) => state.install_facts(id),
        None => (None, None, None, None),
    };

    CtxHolder {
        plugins: state.app_plugins(),
        roots: state.sandbox_roots(),
        settings: state.settings_snapshot(),
        install_id: profile,
        install_dir,
        loader,
        game_version,
        install_name,
    }
}

/// Install (or update) one item now, on the user's say-so.
#[tauri::command]
pub async fn library_install(state: State<'_, AppState>, id: String) -> AppResult<InstallOutcome> {
    let entry = state
        .library
        .get(&id)?
        .ok_or_else(|| AppError::invalid("That item is not in your library."))?;

    let holder = resolve_ctx(&state, &entry);
    let ctx = holder.ctx(&state);

    // Refuse BEFORE downloading anything: a missing game directory should be a
    // sentence, not a failure two hundred megabytes in.
    preflight(&entry, &ctx)?;

    Ok(install_one(&state.library, &entry, &ctx).await)
}

/// Take one item off this machine. The subscription itself is untouched — that
/// is the website's business, and this is the device's.
#[tauri::command]
pub async fn library_uninstall(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<InstallOutcome> {
    let entry = state
        .library
        .get(&id)?
        .ok_or_else(|| AppError::invalid("That item is not in your library."))?;

    let holder = resolve_ctx(&state, &entry);

    Ok(uninstall_one(&state.library, &entry, &holder.ctx(&state), false).await)
}

/// The cloud installs this device has mirrored, as raw payloads.
///
/// Raw because the shape is the API contract's and the frontend already parses
/// it with the mirrored zod schema — re-declaring it in Rust would be a third
/// copy of the same type, and the one most likely to drift.
#[tauri::command]
pub fn library_installs(state: State<'_, AppState>) -> AppResult<Vec<serde_json::Value>> {
    Ok(state
        .library
        .install_payloads()?
        .into_iter()
        .filter_map(|raw| serde_json::from_str(&raw).ok())
        .collect())
}

/// Every install's folder on THIS machine, as `[id, dir]` pairs.
#[tauri::command]
pub fn library_install_dirs(state: State<'_, AppState>) -> AppResult<Vec<(i64, Option<String>)>> {
    state.library.install_local_dirs()
}

/// Where an install lives on THIS machine.
///
/// The one device-local fact about an install, and the reason the mirror table
/// exists at all. Takes a path the user picked in the native folder dialog.
#[tauri::command]
pub fn library_set_install_dir(
    state: State<'_, AppState>,
    id: i64,
    dir: Option<String>,
) -> AppResult<()> {
    if let Some(dir) = &dir {
        let path = PathBuf::from(dir);

        // Stored, never executed — but a directory that does not exist would
        // make every later install fail with a message about the sandbox
        // rather than about the setting the user just changed.
        if !path.is_dir() {
            return Err(AppError::invalid("That folder does not exist."));
        }
    }

    state.library.set_install_local_dir(id, dir.as_deref())?;

    audit!(
        state.audit,
        Info,
        Settings,
        "library.install.dir",
        format!("Install {id} directory set")
    );

    Ok(())
}

/// Which games this build can install for, by slug.
#[tauri::command]
pub fn library_supported_apps(state: State<'_, AppState>) -> Vec<String> {
    state.app_plugins().managed_slugs()
}

/// App-plugin files that failed to load, so Settings → Plugins can say why.
#[tauri::command]
pub fn library_plugin_errors(state: State<'_, AppState>) -> Vec<(String, String)> {
    state.app_plugins().errors().to_vec()
}

/// Re-read `plugins/app/`. Cheap, and the only way to pick up a rule the user
/// just dropped in without restarting.
#[tauri::command]
pub fn library_reload_plugins(state: State<'_, AppState>) -> AppResult<usize> {
    state.reload_app_plugins();

    Ok(state.app_plugins().managed_slugs().len())
}

// ------------------------------------------------------------------- Launch

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchPreview {
    pub plan: LaunchPlan,
    /// A readable one-liner. Display only — nothing consumes it, and it must
    /// never be fed to a shell.
    pub command: String,
}

/// Resolve what launching an install would run, WITHOUT running it.
///
/// This is the whole reason a declarative launcher is acceptable: the user is
/// shown the exact program, arguments and working directory before anything
/// starts. A launcher that cannot show that is one nobody can audit.
#[tauri::command]
pub fn launch_preview(state: State<'_, AppState>, install_id: i64) -> AppResult<LaunchPreview> {
    let plan = state.launch_plan(install_id)?;
    let command = describe(&plan);

    Ok(LaunchPreview { plan, command })
}

/// Start the game.
///
/// The plan is re-resolved here rather than taken from the webview. Accepting a
/// caller-supplied plan would make every one of the checks in
/// `tmc_core::launch` advisory — the frontend would simply hand over the
/// program it wanted.
#[tauri::command]
pub async fn launch_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    install_id: i64,
) -> AppResult<LaunchPreview> {
    let plan = state.launch_plan(install_id)?;
    let command = describe(&plan);

    audit!(
        state.audit,
        Security,
        App,
        "launch.start",
        command.clone(),
        plugin = plan.rule
    );

    if let Some(uri) = &plan.uri {
        /*
         * Handed to the OS opener, which is what resolves a `steam://` link to
         * the installed client. The scheme was checked at load time
         * (`is_allowed_launch_uri`) and the value carries no control
         * characters, so there is nothing here that could become a second
         * argument.
         */
        use tauri_plugin_opener::OpenerExt;

        app.opener()
            .open_url(uri.clone(), None::<&str>)
            .map_err(|e| AppError::internal(format!("could not open {uri}: {e}")))?;
    } else if let Some(program) = &plan.program {
        let mut command_builder = std::process::Command::new(program);

        // An argv VECTOR. There is no shell anywhere in this, so quoting,
        // `;`, backticks and `$( )` are inert bytes to the child.
        command_builder.args(&plan.args);

        if let Some(cwd) = &plan.cwd {
            command_builder.current_dir(cwd);
        }

        for (key, value) in &plan.env {
            command_builder.env(key, value);
        }

        /*
         * Spawned and let go, deliberately: the game outlives the launcher, and
         * holding the handle would make closing the app kill it. The zombie
         * that leaves on unix is reaped by init once this process exits, and by
         * `wait` never being called it costs one PID meanwhile.
         */
        command_builder
            .spawn()
            .map_err(|e| AppError::internal(format!("could not start the game: {e}")))?;
    } else {
        return Err(AppError::invalid(
            "That launch rule names nothing to start.",
        ));
    }

    // Report the play session so the account's install shows a last-played
    // time on every device. Best effort — a failed report is not a failed
    // launch.
    let payload = serde_json::json!({ "id": install_id, "playedSeconds": 0 });

    if let Err(err) = state
        .api
        .request(
            tmc_core::api::Method::PATCH,
            "/installs",
            Some(payload),
            true,
        )
        .await
    {
        tracing::warn!("could not report launch: {}", err.detail());
    }

    Ok(LaunchPreview { plan, command })
}

/// Does this game have a launch rule at all? Drives whether the button exists.
#[tauri::command]
pub fn launch_available(state: State<'_, AppState>, slug: String) -> bool {
    state.app_plugins().launch_for(&slug, None).is_some()
}

/// The rule files loaded for one game, for Settings → Plugins.
#[tauri::command]
pub fn library_rules_for(state: State<'_, AppState>, slug: String) -> Vec<RuleInfo> {
    state
        .app_plugins()
        .for_slug(&slug)
        .iter()
        .map(|f| RuleInfo {
            source: f.source.clone(),
            kind: f.kind.as_str().to_string(),
            label: f.label.clone(),
            description: f.description.clone(),
            extensions: f.r#match.extensions.clone(),
            loaders: f.r#match.loaders.clone(),
            manages: f.kind != AppPluginKind::Launch,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleInfo {
    pub source: String,
    pub kind: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub extensions: Vec<String>,
    pub loaders: Vec<String>,
    pub manages: bool,
}
