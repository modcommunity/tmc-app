//! **Games TMC publishes**: installing them, keeping them current, starting
//! them.
//!
//! The command surface over [`tmc_core::games`]. Everything the Library's
//! "TMC Games" section can do is here, and the two rules at the top of
//! `commands/mod.rs` hold without an exception:
//!
//!   * **Nothing here takes a path.** The install root comes from Tauri's path
//!     resolver, the version directory from the build the SITE resolved, and
//!     the executable from that build's own declaration put through the jail's
//!     lexical check. A caller names an app id.
//!   * **Nothing here takes a URL.** `game_install` asks the API where the
//!     build is; there is no `game_install(url)`, for the same reason there is
//!     no `download_start(url, path)`.
//!
//! WHY A LAUNCH NAMES A SERVER ID AND NOT AN ADDRESS
//! ------------------------------------------------
//! The address is read from the API here, as `play_connect` already does it.
//! A caller-supplied `host:port` would be a caller choosing what a process this
//! app starts is pointed at — and while a game client dialling a stranger's box
//! is a smaller problem than a plugin writing to one, it is the same shape, and
//! the address is one lookup away.
//!
//! WHY AN INSTALL IS NOT AWAITED BY THE UI
//! --------------------------------------
//! It is: `game_install` runs to completion and the dialog awaits it. The
//! PROGRESS, though, comes from the download queue's own events — the build is
//! enqueued like every other download, so the existing queue screen shows it,
//! pauses it and rate-limits it without learning what a game is.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use tmc_core::error::{AppError, AppResult};
use tmc_core::games::{BuildPlatform, GameStatus, LaunchContext, NativeBuild};
use tmc_core::library::db::InstalledGame;
use tmc_core::session::{Session, SessionSpec};

use crate::state::AppState;

/// What this machine can install, so the UI can say why it cannot.
///
/// Two separate facts, deliberately. "This app publishes nothing for your
/// architecture" can never change for the person reading it; "games are
/// installed through your platform's own store" is a statement about Android
/// and iOS refusing to execute what an app wrote into its own container. A
/// single `false` would have made those one message, and neither of the two
/// sentences that message could carry would be true for both.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamePlatformInfo {
    /// The target this machine asks for, or null for one nothing is built for.
    pub platform: Option<String>,
    /// Whether the app can put a build on this disk and start it.
    pub installable: bool,
}

#[tauri::command]
pub fn games_platform(state: State<'_, AppState>) -> GamePlatformInfo {
    let platform = state.games.platform();

    GamePlatformInfo {
        platform: platform.map(BuildPlatform::as_str).map(str::to_string),
        installable: platform.is_some_and(BuildPlatform::installable),
    }
}

/// Everything installed on this device, newest information first.
#[tauri::command]
pub fn games_list(state: State<'_, AppState>) -> AppResult<Vec<InstalledGame>> {
    state.games.installed()
}

/// What the site has for this machine, or null for "nothing to install".
#[tauri::command]
pub async fn games_available(
    state: State<'_, AppState>,
    app_id: i64,
) -> AppResult<Option<NativeBuild>> {
    state.games.resolve(app_id).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRequest {
    pub app_id: i64,
    /// What to call it while it downloads. A LABEL — the row's real name comes
    /// from the catalogue, and nothing is keyed on this.
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
}

/// Install, or update in place. One command, because an update is an install
/// whose row already exists — see `Games::install`.
#[tauri::command]
pub async fn game_install(
    state: State<'_, AppState>,
    request: InstallRequest,
) -> AppResult<InstalledGame> {
    /*
     * Refused while it is running. The new files would land on top of ones the
     * running process has mapped, which on Windows fails outright and on Unix
     * succeeds and leaves a process running code that is no longer on disk.
     */
    if state.sessions.running_for_app(request.app_id) {
        return Err(AppError::invalid(
            "That game is running. Close it before installing an update.",
        ));
    }

    state
        .games
        .install(request.app_id, &request.name, request.slug.as_deref())
        .await
}

#[tauri::command]
pub async fn game_uninstall(state: State<'_, AppState>, app_id: i64) -> AppResult<()> {
    if state.sessions.running_for_app(app_id) {
        return Err(AppError::invalid(
            "That game is running. Close it before removing it.",
        ));
    }

    state.games.uninstall(app_id)
}

#[tauri::command]
pub fn game_set_auto_update(
    state: State<'_, AppState>,
    app_id: i64,
    enabled: bool,
) -> AppResult<()> {
    state.games.set_auto_update(app_id, enabled)
}

/// Check every installed game against the site, and say what is out of date.
///
/// Reports rather than acts, even for a game with auto-update on. The automatic
/// pass is `games::updates::run`, which the sync loop drives; this is the button
/// somebody presses, and a button that silently downloaded four gigabytes
/// because the row happened to have a flag set would be a button nobody presses
/// twice.
#[tauri::command]
pub async fn games_check_updates(state: State<'_, AppState>) -> AppResult<Vec<GameStatus>> {
    tmc_core::games::updates::check(&state.games).await
}

/// Bring every installed game that asked to be kept current forward.
///
/// Called on the library's sync pass, which is the device's own "am I online
/// now?" heartbeat — the same place outstanding play time is drained, and for
/// the same reason: it is already the moment the device has an API to talk to.
///
/// Bounded by the rules in `games::updates`: the install must have asked for
/// this, the game must not be running, and a version that cannot be ordered is
/// never an update. A game that fails leaves the version it already had on disk
/// and working, because the new build lands in its own directory and the row is
/// only repointed once the entry is verified.
#[tauri::command]
pub async fn games_auto_update(
    state: State<'_, AppState>,
) -> AppResult<tmc_core::games::updates::GameUpdateReport> {
    let sessions = std::sync::Arc::clone(&state.sessions);

    tmc_core::games::updates::run(&state.games, &move |app_id| {
        sessions.running_for_app(app_id)
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameLaunchRequest {
    pub app_id: i64,
    /// Join this server. Absent starts the game at its own menu.
    #[serde(default)]
    pub server_id: Option<i64>,
    /// The values the user chose for the app's declared launch options.
    #[serde(default)]
    pub options: Option<serde_json::Value>,
    /// The viewer's language tag, for a build whose arguments name `{locale}`.
    ///
    /// From the caller because locale is an ACCOUNT setting and this side has
    /// none — the app-local settings are the machine facts, deliberately. It is
    /// a display string that goes through the same control-character check
    /// every other substituted value does, so the worst a wrong one produces is
    /// a game in the wrong language.
    #[serde(default)]
    pub locale: Option<String>,
}

/// Start an installed game, optionally into a server.
#[tauri::command]
pub async fn game_launch(
    app: AppHandle,
    state: State<'_, AppState>,
    request: GameLaunchRequest,
) -> AppResult<Session> {
    let Some(row) = state.games.get(request.app_id)? else {
        return Err(AppError::invalid("That game is not installed."));
    };

    /*
     * A second copy would open files the first has mapped, and the play clock
     * for one game would be able to exceed wall-clock time. The same refusal a
     * sandbox launch carries, for the same two reasons.
     */
    if state.sessions.running_for_app(request.app_id) {
        return Err(AppError::invalid("That game is already running."));
    }

    let mut ctx = LaunchContext {
        app_id: request.app_id,
        app_slug: row.slug.clone(),
        locale: request.locale.clone(),
        ..Default::default()
    };

    if let Some(server_id) = request.server_id {
        let (host, port) = resolve_server(&state, server_id).await?;

        ctx.server_id = Some(server_id);
        ctx.host = Some(host);
        ctx.port = port;
    }

    if let Some(serde_json::Value::Object(map)) = request.options {
        for (key, value) in map {
            let text = match value {
                serde_json::Value::String(s) => s,
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                // An object or an array contributes nothing, exactly as it
                // does to a sandbox's `optionArgs`. There is no shell and no
                // serialisation a game could be relied on to parse.
                _ => continue,
            };

            ctx.options.insert(key, text);
        }
    }

    let plan = state.games.plan(&row, &ctx)?;

    let session = crate::spawn::run(
        &app,
        &state,
        &plan,
        SessionSpec {
            app_id: Some(request.app_id),
            app_slug: row.slug.clone(),
            label: row.name.clone(),
            sandbox_id: None,
            // A TMC game is not a subscription, so there is no install row on
            // the account to report its play time against. It is still measured
            // locally and still written to `game_session`.
            install_id: None,
        },
    )?;

    Ok(session)
}

/// The address of one server, read here rather than accepted from the caller.
///
/// Null host is the owner having hidden the network details, which is a refusal
/// rather than a launch into nothing: a client started with no address shows its
/// own "could not connect" and the person who pressed Join has no way to tell
/// that from a server being down.
async fn resolve_server(state: &AppState, server_id: i64) -> AppResult<(String, Option<u16>)> {
    let detail = state
        .api
        .request(
            tmc_core::api::Method::GET,
            &format!("/content/server/{server_id}"),
            None,
            false,
        )
        .await?;

    let summary = detail.get("summary").unwrap_or(&detail);

    let server = summary
        .get("server")
        .ok_or_else(|| AppError::invalid("That is not a server."))?;

    let host = server
        .get("host")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|h| !h.is_empty())
        .ok_or_else(|| {
            AppError::invalid("That server does not publish an address to connect to.")
        })?;

    let port = server
        .get("port")
        .and_then(serde_json::Value::as_u64)
        .and_then(|p| u16::try_from(p).ok());

    Ok((host, port))
}
