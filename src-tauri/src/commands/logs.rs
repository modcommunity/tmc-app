use tauri::State;

use tmc_core::audit;
use tmc_core::error::AppResult;
use tmc_core::logging::{LogEntry, LogLevel};

use crate::state::AppState;

#[tauri::command]
pub fn log_read(
    state: State<'_, AppState>,
    limit: Option<usize>,
    min_level: Option<LogLevel>,
) -> AppResult<Vec<LogEntry>> {
    state.audit.read(limit.unwrap_or(500), min_level)
}

#[tauri::command]
pub fn log_clear(state: State<'_, AppState>) -> AppResult<()> {
    state.audit.clear()?;

    // Written AFTER the clear, so the record that the log was cleared survives
    // it. A log you can erase without trace is not an audit trail.
    audit!(
        state.audit,
        Security,
        App,
        "log.clear",
        "Activity log cleared by the user"
    );

    Ok(())
}

#[tauri::command]
pub fn log_path(state: State<'_, AppState>) -> String {
    state.paths.audit_file().display().to_string()
}
