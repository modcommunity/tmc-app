//! RCON's IPC surface.
//!
//! **No command here returns a password**, and there is no shape one could take
//! that would. A password goes IN on create or on change, and after that the
//! only thing the webview can name is a server id — the decryption and the
//! connection both happen inside `tmc_core::rcon::exec_saved`, which takes an
//! id and returns output.
//!
//! `rcon_exec` is the one command in the app that sends a user's text to a
//! third party's machine, so every call is logged with its output on the
//! server's own history, and every connection is audited at Security level.

use std::time::Duration;

use tauri::State;

use tmc_core::error::{AppError, AppResult};
use tmc_core::rcon::store::{NewRconServer, RconHistoryRow, RconServer};
use tmc_core::rcon::{RconProtocol, RconReply, RconTarget, DEFAULT_TIMEOUT_MS};

use crate::state::AppState;

/// The longest a single command may take. A server that has not answered by
/// then is one the user needs told about, not waited on.
const MAX_TIMEOUT_MS: u64 = 30_000;

/// Saved servers. Never their passwords — [`RconServer`] has no field for one.
#[tauri::command]
pub fn rcon_list(state: State<'_, AppState>) -> AppResult<Vec<RconServer>> {
    state.library.rcon_list()
}

#[tauri::command]
pub fn rcon_create(state: State<'_, AppState>, server: NewRconServer) -> AppResult<i64> {
    let cipher = state.cipher()?;

    let id = state.library.rcon_create(cipher, &server)?;

    tmc_core::audit!(
        state.audit,
        Security,
        App,
        "rcon.save",
        format!("{} ({}:{})", server.name, server.host, server.port)
    );

    Ok(id)
}

#[tauri::command]
pub fn rcon_update(
    state: State<'_, AppState>,
    id: i64,
    name: String,
    host: String,
    port: u16,
) -> AppResult<()> {
    state.library.rcon_rename(id, &name, &host, port)
}

/// Change or clear the stored password.
///
/// Separate from [`rcon_update`] so a rename does not have to carry a password
/// through the webview to keep it.
#[tauri::command]
pub fn rcon_set_password(
    state: State<'_, AppState>,
    id: i64,
    password: Option<String>,
) -> AppResult<()> {
    let cipher = state.cipher()?;

    state
        .library
        .rcon_set_password(cipher, id, password.as_deref())?;

    tmc_core::audit!(
        state.audit,
        Security,
        App,
        "rcon.password",
        format!(
            "server {id}: password {}",
            if password.is_some() { "set" } else { "cleared" }
        )
    );

    Ok(())
}

#[tauri::command]
pub async fn rcon_delete(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    if let Some(server) = state.library.rcon_get(id)? {
        state
            .rcon
            .disconnect(&target_for(&server, DEFAULT_TIMEOUT_MS))
            .await;
    }

    state.library.rcon_delete(id)
}

/// Open a session, so a wrong password is reported before somebody types into a
/// pane that looks live.
#[tauri::command]
pub async fn rcon_connect(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    let cipher = state.cipher()?;

    tmc_core::rcon::connect_saved(
        &state.library,
        cipher,
        &state.rcon,
        id,
        Duration::from_millis(DEFAULT_TIMEOUT_MS),
    )
    .await
}

#[tauri::command]
pub async fn rcon_disconnect(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    if let Some(server) = state.library.rcon_get(id)? {
        state
            .rcon
            .disconnect(&target_for(&server, DEFAULT_TIMEOUT_MS))
            .await;
    }

    Ok(())
}

#[tauri::command]
pub async fn rcon_is_connected(state: State<'_, AppState>, id: i64) -> AppResult<bool> {
    let Some(server) = state.library.rcon_get(id)? else {
        return Ok(false);
    };

    Ok(state
        .rcon
        .is_connected(&target_for(&server, DEFAULT_TIMEOUT_MS))
        .await)
}

/// Run one command.
#[tauri::command]
pub async fn rcon_exec(
    state: State<'_, AppState>,
    id: i64,
    command: String,
    timeout_ms: Option<u64>,
) -> AppResult<RconReply> {
    if command.trim().is_empty() {
        return Err(AppError::invalid("Type a command."));
    }

    let cipher = state.cipher()?;

    let timeout = Duration::from_millis(
        timeout_ms
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1_000, MAX_TIMEOUT_MS),
    );

    tmc_core::rcon::exec_saved(&state.library, cipher, &state.rcon, id, &command, timeout).await
}

/// What has been run on this server, newest first.
#[tauri::command]
pub fn rcon_history(
    state: State<'_, AppState>,
    id: i64,
    limit: Option<usize>,
) -> AppResult<Vec<RconHistoryRow>> {
    state.library.rcon_history(id, limit.unwrap_or(200))
}

#[tauri::command]
pub fn rcon_clear_history(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    state.library.rcon_clear_history(id)
}

/// Which protocol a game speaks, for the "add server" form's default.
///
/// A guess from the game's query protocol, because a game that answers A2S
/// almost always speaks Valve RCON and one that answers Frostbite always speaks
/// Frostbite RCON. It is only the form's initial value — the field stays
/// editable, because some servers run a proxy that speaks the other one.
#[tauri::command]
pub fn rcon_suggest_protocol(query_protocol: Option<String>) -> RconProtocol {
    match query_protocol.as_deref() {
        Some("FROSTBITE") => RconProtocol::Frostbite,
        _ => RconProtocol::Source,
    }
}

fn target_for(server: &RconServer, timeout_ms: u64) -> RconTarget {
    RconTarget {
        host: server.host.clone(),
        port: server.port,
        protocol: server.protocol,
        timeout: Duration::from_millis(timeout_ms),
    }
}
