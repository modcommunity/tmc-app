use serde_json::Value;
use tauri::State;

use tmc_core::audit;
use tmc_core::error::AppResult;
use tmc_core::settings::AppSettings;

use crate::state::AppState;

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> AppSettings {
    state.settings.get()
}

#[tauri::command]
pub fn settings_patch(state: State<'_, AppState>, patch: Value) -> AppResult<AppSettings> {
    let next = state.settings.patch(patch)?;

    // Logging verbosity is the one setting with an immediate side effect.
    state.audit.set_verbose(next.verbose_logging);

    audit!(
        state.audit,
        Info,
        Settings,
        "settings.update",
        "App settings changed"
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
