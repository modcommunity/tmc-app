//! **Games that are running**, and what happened to the ones that finished.
//!
//! Everything here reads from [`tmc_core::session::Sessions`] (the live half)
//! and the library database's `game_session` table (the durable half). Nothing
//! here starts anything — a launch goes through [`crate::commands::library`] or
//! [`crate::commands::sandbox`], which are the two places that hold a resolved
//! [`LaunchPlan`] and the checks that produced it.
//!
//! [`LaunchPlan`]: tmc_core::launch::LaunchPlan
//!
//! WHY PLAYTIME IS FLUSHED RATHER THAN POSTED
//! -----------------------------------------
//! A game is very often played offline — a laptop on a train is the case the
//! feature is for. Reporting at the moment a session ends would mean playtime
//! that only counts when the network happened to be up, which is playtime that
//! silently goes missing and cannot be recovered because nothing kept it.
//!
//! So a finished session is WRITTEN by the supervisor and marked unreported,
//! and [`sessions_flush`] sends whatever is outstanding whenever the device
//! next has an API to talk to. A row is marked reported only after the server
//! has accepted it, so a flush that fails halfway repeats rather than drops.

use serde::Serialize;
use tauri::State;

use tmc_core::error::AppResult;
use tmc_core::library::{PlayTotals, SessionRow};
use tmc_core::session::Session;

use crate::state::AppState;

/// What is running right now.
#[tauri::command]
pub fn session_running(state: State<'_, AppState>) -> Vec<Session> {
    state.sessions.running()
}

/// Recent launches, from the durable history.
///
/// The database rather than the in-memory recent list, because this is what the
/// Library shows and it has to survive a restart — "what did I play last" is
/// exactly the question a freshly-opened app is asked.
#[tauri::command]
pub fn session_history(
    state: State<'_, AppState>,
    app_id: Option<i64>,
    limit: Option<usize>,
) -> AppResult<Vec<SessionRow>> {
    state.library.session_history(app_id, limit.unwrap_or(50))
}

/// Stop a running game.
///
/// Kills rather than asking politely, and [`tmc_core::session::Sessions::stop`]
/// explains why there is no portable middle ground. The session closes when the
/// process actually goes, not when this returns — so a kill that is refused
/// leaves the game listed as running, which is true.
#[tauri::command]
pub fn session_stop(state: State<'_, AppState>, id: u64) -> AppResult<()> {
    let label = state
        .sessions
        .running()
        .into_iter()
        .find(|s| s.id == id)
        .map(|s| s.label)
        .unwrap_or_else(|| "a game".to_string());

    tmc_core::audit!(
        state.audit,
        Info,
        App,
        "session.stop",
        format!("stopped {label}")
    );

    state.sessions.stop(id)
}

/// The last lines a session's process printed.
///
/// The whole crash-report story: a game that exits in two seconds has almost
/// always said why, and without a captured pipe that goes to a console nobody
/// attached. Bounded from the END, because the interesting part of a crash is
/// the last thing said before it.
#[tauri::command]
pub fn session_log(state: State<'_, AppState>, id: u64, lines: Option<usize>) -> AppResult<String> {
    state
        .sessions
        .log_tail(id, lines.unwrap_or(400).clamp(1, 5_000))
}

/// Play totals, per game and per sandbox.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaytimeSummary {
    /// Keyed by TMC app id, as a string — a JSON object's keys are strings and
    /// a numeric key would round-trip as one anyway.
    pub by_app: std::collections::BTreeMap<String, PlayTotals>,
    pub by_sandbox: std::collections::BTreeMap<String, PlayTotals>,
}

#[tauri::command]
pub fn playtime_summary(state: State<'_, AppState>) -> AppResult<PlaytimeSummary> {
    let by_app = state
        .library
        .playtime_by_app()?
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    let by_sandbox = state
        .library
        .playtime_by_sandbox()?
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    Ok(PlaytimeSummary { by_app, by_sandbox })
}

/// What one flush did.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlushReport {
    pub reported: usize,
    pub seconds: i64,
    /// Sessions that were outstanding and could not be sent this time.
    pub deferred: usize,
}

/// Send outstanding playtime to the account.
///
/// Called after a session ends and on the library's own sync pass, so a device
/// that was offline when a game closed catches up the next time it is not.
///
/// **One request per install, not per session.** The API's field is "seconds
/// since the last report", so several sessions for one install collapse into a
/// single number — which is also what stops a weekend of offline play becoming
/// forty requests on the next connection.
#[tauri::command]
pub async fn sessions_flush(state: State<'_, AppState>) -> AppResult<FlushReport> {
    /*
     * One at a time.
     *
     * The rows are read, sent, and only THEN marked reported, so two
     * overlapping runs read the same rows and each report the same seconds
     * against the same install. The one caller fires this without awaiting it
     * — a playtime report must not hold up a library sync — so its own
     * re-entrancy guard is released long before this finishes.
     *
     * A refused flush is not an error: whatever is outstanding is still
     * outstanding, and the run already in progress is about to send it.
     */
    if !state.flush_begin() {
        return Ok(FlushReport::default());
    }

    let result = flush_inner(&state).await;

    state.flush_end();

    result
}

async fn flush_inner(state: &AppState) -> AppResult<FlushReport> {
    let pending = state.library.sessions_unreported(200)?;

    if pending.is_empty() {
        return Ok(FlushReport::default());
    }

    // Grouped by install so the report is one request per install rather than
    // one per session.
    let mut totals: std::collections::BTreeMap<i64, (i64, Vec<i64>)> =
        std::collections::BTreeMap::new();

    for row in &pending {
        let Some(install_id) = row.install_id else {
            continue;
        };

        let slot = totals.entry(install_id).or_insert((0, Vec::new()));

        slot.0 += row.seconds;
        slot.1.push(row.id);
    }

    let mut report = FlushReport::default();

    for (install_id, (seconds, ids)) in totals {
        /*
         * Clamped to the contract's own ceiling. `InstallUpdateRequest` accepts
         * at most 86,400 — a day — and a device that was offline for a week can
         * legitimately have more than that outstanding. Sending it unclamped
         * would 400 the whole request and the rows would never clear, so the
         * excess is dropped and the sessions are still marked reported.
         *
         * Losing the overflow is the right trade: the alternative is a queue
         * that can never drain, and playtime is a statistic rather than a
         * ledger.
         */
        let payload = serde_json::json!({
            "id": install_id,
            "playedSeconds": seconds.clamp(0, 86_400),
        });

        match state
            .api
            .request(
                tmc_core::api::Method::PATCH,
                "/installs",
                Some(payload),
                true,
            )
            .await
        {
            Ok(_) => {
                state.library.sessions_mark_reported(&ids)?;

                report.reported += ids.len();
                report.seconds += seconds;
            }
            Err(err) => {
                /*
                 * Left unreported, deliberately. The next flush picks it up —
                 * that is the entire reason the column exists rather than the
                 * report happening inline when the game exits.
                 */
                tracing::warn!("could not report playtime: {}", err.detail());

                report.deferred += ids.len();
            }
        }
    }

    Ok(report)
}
