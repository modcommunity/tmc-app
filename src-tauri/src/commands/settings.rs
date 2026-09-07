use std::path::PathBuf;

use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use tmc_core::audit;
use tmc_core::error::AppResult;
use tmc_core::settings::AppSettings;

use crate::state::AppState;

/// The directories a jail root must not BE, or CONTAIN.
///
/// Assembled here rather than in `tmc-core` because every one of them comes
/// from Tauri's path resolver, which is the whole reason `paths.rs` exists. The
/// core crate takes them as plain data so its policy stays testable on a runner
/// with no window — see `tmc_core::anchor`.
///
/// Home is included explicitly. On every current platform the app's data
/// directory already sits under it, so the containment rule would refuse home
/// anyway; naming it means the refusal does not quietly depend on that staying
/// true if a platform moves its app-data location.
fn protected_dirs(app: &AppHandle, state: &AppState) -> Vec<PathBuf> {
    let mut dirs = vec![
        state.paths.data.clone(),
        state.paths.logs.clone(),
        state.paths.cache.clone(),
        state.paths.plugins.clone(),
    ];

    if let Ok(home) = app.path().home_dir() {
        dirs.push(home);
    }

    dirs
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> AppSettings {
    state.settings.get()
}

/// Every setting EXCEPT the two jail roots.
///
/// `gameDirs` and `downloadDir` are refused by `SettingsStore::patch` itself,
/// not filtered here, so the gate holds for any caller rather than only for
/// this command. They go through the two setters below.
#[tauri::command]
pub fn settings_patch(state: State<'_, AppState>, patch: Value) -> AppResult<AppSettings> {
    let next = state.settings.patch(patch)?;

    // Two settings have an immediate side effect rather than being read where
    // they are used, and both are pushed here so a change takes hold without a
    // restart.
    state.audit.set_verbose(next.verbose_logging);

    /*
     * The plugin gate. A user turning this ON expects the next install run to
     * be refused, not the one after their next launch — and a user turning it
     * OFF because a plugin they trust is unsigned expects that plugin to work
     * immediately.
     */
    state
        .plugins
        .set_require_signed(next.require_signed_plugins);

    audit!(
        state.audit,
        Info,
        Settings,
        "settings.update",
        "App settings changed"
    );

    Ok(next)
}

/// Point a game's install folder at `dir`, or clear it with `null`.
///
/// This is the ONLY way that value moves. It exists as its own command because
/// it is not a preference: the path becomes the anchor of the plugin jail, so
/// an installer plugin holding `{gameDir, "", write}` can write anywhere
/// beneath it. `tmc_core::anchor::validate_root` decides whether a given
/// directory is an acceptable anchor, and its module header is honest about
/// what that check does and does not defend.
///
/// Audited at **Security** level, so the entry is written even when the user
/// has logging turned off. Changing where a plugin may write is exactly the
/// class of event that switch must not be able to hide.
#[tauri::command]
pub fn settings_set_game_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: String,
    dir: Option<String>,
) -> AppResult<AppSettings> {
    let protected = protected_dirs(&app, &state);

    let (next, stored) = state
        .settings
        .set_game_dir(&app_id, dir.as_deref(), &protected)?;

    /*
     * The CANONICAL path is logged, not the string that arrived. They differ
     * whenever the input held a symlink or a `..`, and the log has to record
     * the root that plugins will actually be jailed to — which is the resolved
     * one — or it documents an intention rather than a fact.
     */
    audit!(
        state.audit,
        Security,
        Settings,
        "settings.gameDir",
        match &stored {
            Some(path) => format!("Game {app_id} install folder set to {path}"),
            None => format!("Game {app_id} install folder cleared"),
        }
    );

    Ok(next)
}

/// Where installers put downloads before unpacking them, or `null` for the
/// app's own cache. Same reasoning and the same validation as a game folder: a
/// plugin's `downloads` grant resolves beneath it.
#[tauri::command]
pub fn settings_set_download_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    dir: Option<String>,
) -> AppResult<AppSettings> {
    let protected = protected_dirs(&app, &state);

    let (next, stored) = state
        .settings
        .set_download_dir(dir.as_deref(), &protected)?;

    audit!(
        state.audit,
        Security,
        Settings,
        "settings.downloadDir",
        match &stored {
            Some(path) => format!("Download folder set to {path}"),
            None => "Download folder reset to the app cache".to_string(),
        }
    );

    Ok(next)
}

#[tauri::command]
pub fn settings_reset(state: State<'_, AppState>) -> AppResult<AppSettings> {
    let fresh = state.settings.reset()?;
    state.audit.set_verbose(fresh.verbose_logging);

    audit!(
        state.audit,
        Security,
        Settings,
        "settings.reset",
        "App settings reset to defaults"
    );

    Ok(fresh)
}
