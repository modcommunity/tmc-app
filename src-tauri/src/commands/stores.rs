//! Stores: named download/install locations. See `tmc_core::library::stores`.
//!
//! `storage_create` takes a filesystem path from the webview. That is a
//! documented exception to the rule in this module's parent: the path is the
//! folder the user just picked in the app's own folder picker, and it is
//! validated (absolute, writable, not inside the app's data, not nested in
//! another store, and an acceptable jail anchor) before anything is stored.
//! Making a store the default goes through the same validating setter as
//! Settings, so the plugin jail's `downloads` anchor has one gate.

use tauri::{AppHandle, State};

use tmc_core::audit;
use tmc_core::error::{AppError, AppResult};
use tmc_core::library::stores::{Store, StoreKind};

use crate::commands::settings::protected_dirs;
use crate::state::AppState;

/// Bring the table in line with settings. Cheap; run before every read so the
/// list is right even after a change made in Settings.
fn sync(state: &AppState) -> AppResult<()> {
    let settings = state.settings.get();

    state.library.store_sync_default(
        settings.download_dir.as_deref(),
        &state.paths.cache.join("downloads"),
    )
}

/// Run at startup and exit by `lib.rs`.
pub fn startup_sync(state: &AppState) {
    if let Err(e) = sync(state) {
        tracing::warn!("stores: sync failed: {}", e.detail());
    }

    sweep(state, "startup");
}

pub fn sweep(state: &AppState, when: &str) {
    match state.library.store_sweep_temporary() {
        Ok(report) if report.removed > 0 || report.failed > 0 => {
            tracing::info!(
                "stores: {when} sweep removed {} entries, {} stores failed",
                report.removed,
                report.failed
            );
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("stores: {when} sweep failed: {}", e.detail()),
    }
}

#[tauri::command]
pub async fn storage_list(state: State<'_, AppState>) -> AppResult<Vec<Store>> {
    sync(&state)?;

    let library = state.library.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let mut stores = library.store_list()?;

        stores.iter_mut().for_each(Store::fill_stats);

        Ok(stores)
    })
    .await
    .map_err(|e| AppError::internal(format!("store scan failed: {e}")))?
}

#[tauri::command]
pub fn storage_create(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    path: String,
    kind: StoreKind,
) -> AppResult<i64> {
    let protected = protected_dirs(&app, &state);
    let app_owned = vec![
        state.paths.data.clone(),
        state.paths.logs.clone(),
        state.paths.cache.clone(),
        state.paths.plugins.clone(),
    ];

    let id = state
        .library
        .store_create(&name, &path, kind, &protected, &app_owned)?;

    audit!(
        state.audit,
        Security,
        Settings,
        "store.create",
        format!(
            "Store \"{}\" ({:?}) created at {}",
            name.trim(),
            kind,
            path.trim()
        )
    );

    Ok(id)
}

#[tauri::command]
pub fn storage_rename(state: State<'_, AppState>, id: i64, name: String) -> AppResult<()> {
    state.library.store_rename(id, &name)
}

#[tauri::command]
pub fn storage_delete(state: State<'_, AppState>, id: i64, confirm_wipe: bool) -> AppResult<()> {
    sync(&state)?;

    let game_dirs: Vec<String> = state.settings.get().game_dirs.values().cloned().collect();

    state.library.store_delete(id, confirm_wipe, &game_dirs)?;

    audit!(
        state.audit,
        Security,
        Settings,
        "store.delete",
        format!("Store {id} deleted")
    );

    Ok(())
}

#[tauri::command]
pub fn storage_set_default(app: AppHandle, state: State<'_, AppState>, id: i64) -> AppResult<()> {
    let target = state.library.store_default_target(id)?;
    let protected = protected_dirs(&app, &state);

    let (_, stored) = state
        .settings
        .set_download_dir(target.as_deref(), &protected)?;

    audit!(
        state.audit,
        Security,
        Settings,
        "settings.downloadDir",
        match &stored {
            Some(path) => format!("Download folder set to {path} (store {id})"),
            None => "Download folder reset to the app cache".to_string(),
        }
    );

    sync(&state)
}
