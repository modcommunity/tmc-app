//! Live server queries and the latency history they feed.
//!
//! The browser calls [`query_servers`] with whatever is currently on screen. It
//! is one command rather than one per card for two reasons: a fifty-row grid
//! would otherwise be fifty IPC round trips per refresh, and the concurrency
//! limit has to be enforced somewhere that can see the whole batch. A phone's
//! radio and most home routers handle fifty simultaneous UDP sockets badly.

use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::State;

use tmc_core::error::AppResult;
use tmc_core::net::latency::{LatencySeries, LatencyStore};
use tmc_core::net::query::{query, QueryProtocol, QueryTarget, ServerQueryResult};
use tmc_core::net::{tcp_ping, PingResult};

use crate::state::AppState;

/// Servers accepted in one batch. Comfortably more than a viewport holds.
const MAX_BATCH: usize = 64;

/// Concurrency ceiling, whatever the settings say.
const MAX_CONCURRENCY: usize = 32;

/// One row's worth of "what should I query, and how".
///
/// The protocol list arrives from the API as the game's declared protocols in
/// preference order; the app picks the first it implements natively.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRequest {
    /// Opaque id the caller uses to match results back to rows. Never
    /// interpreted here.
    pub id: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub query_port: Option<u16>,
    #[serde(default)]
    pub protocols: Vec<QueryProtocol>,
    #[serde(default)]
    pub port_offset: i32,
    #[serde(default)]
    pub swap_game_port: bool,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub want_players: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryOutcome {
    pub id: String,
    /// The latency-history key, so the caller can ask for the series later
    /// without reconstructing it (and getting the casing wrong).
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ServerQueryResult>,
    /// Present when the query failed. A human-readable sentence; the row shows
    /// "offline" and this as a tooltip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Pick the protocol to speak.
///
/// The game may declare several — an app that moved from GameSpy to A2S, say —
/// and the first one this build implements natively wins. Falling through to
/// `TcpOnly` rather than failing means a game whose protocol we do not speak
/// still gets a real latency figure, which is most of what the browser renders.
fn choose(protocols: &[QueryProtocol]) -> QueryProtocol {
    protocols
        .iter()
        .copied()
        .find(|p| p.is_native())
        .unwrap_or(QueryProtocol::TcpOnly)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Query a batch of servers, bounded by the user's concurrency setting.
#[tauri::command]
pub async fn query_servers(
    state: State<'_, AppState>,
    requests: Vec<QueryRequest>,
) -> AppResult<Vec<QueryOutcome>> {
    let concurrency =
        usize::from(state.settings.get().latency_concurrency).clamp(1, MAX_CONCURRENCY);

    let batch: Vec<QueryRequest> = requests.into_iter().take(MAX_BATCH).collect();

    let mut pending = batch.into_iter();
    let mut running = FuturesUnordered::new();
    let mut out: Vec<QueryOutcome> = Vec::new();

    /*
     * A hand-rolled window rather than `buffer_unordered`: the results are
     * recorded into the latency store as they land, and doing that inside the
     * stream would need the store borrowed across every future. Filling the
     * window and topping it up as each finishes gives the same concurrency with
     * a plain loop.
     */
    for _ in 0..concurrency {
        match pending.next() {
            Some(request) => running.push(run_one(request)),
            None => break,
        }
    }

    while let Some(outcome) = running.next().await {
        // Record BEFORE pushing, so a caller reading the series in the same
        // tick sees this sample.
        state.latency.record(
            &outcome.key,
            now_ms(),
            outcome.result.as_ref().map(|r| r.rtt_ms),
        );

        out.push(outcome);

        if let Some(request) = pending.next() {
            running.push(run_one(request));
        }
    }

    Ok(out)
}

async fn run_one(request: QueryRequest) -> QueryOutcome {
    let protocol = choose(&request.protocols);

    let target = QueryTarget {
        host: request.host.clone(),
        game_port: request.port,
        query_port: request.query_port,
        port_offset: request.port_offset,
        swap_game_port: request.swap_game_port,
        protocol: Some(protocol),
        timeout_ms: request.timeout_ms,
        want_players: request.want_players,
    };

    /*
     * The history key uses the GAME port, not the resolved query port. The
     * query port can change when a game's config is corrected, and a graph that
     * silently starts a new series because an admin fixed an offset is worse
     * than one that spans the change.
     */
    let key = LatencyStore::key(&request.host, request.port);

    match query(&target).await {
        Ok(result) => QueryOutcome {
            id: request.id,
            key,
            result: Some(result),
            error: None,
        },
        Err(err) => QueryOutcome {
            id: request.id,
            key,
            result: None,
            error: Some(err.to_string()),
        },
    }
}

/// One server, in full — the view page's live panel, with the roster.
#[tauri::command]
pub async fn query_server(
    state: State<'_, AppState>,
    request: QueryRequest,
) -> AppResult<QueryOutcome> {
    let outcome = run_one(request).await;

    state.latency.record(
        &outcome.key,
        now_ms(),
        outcome.result.as_ref().map(|r| r.rtt_ms),
    );

    Ok(outcome)
}

/// The latency history for a set of servers, for the sparklines and the graph.
#[tauri::command]
pub fn latency_series(state: State<'_, AppState>, keys: Vec<String>) -> Vec<LatencySeries> {
    state.latency.many(&keys)
}

#[tauri::command]
pub fn latency_clear(state: State<'_, AppState>) {
    state.latency.clear();
}

/// A bare TCP handshake, for callers with no protocol at all.
#[tauri::command]
pub async fn ping_server(
    state: State<'_, AppState>,
    host: String,
    port: u16,
    timeout_ms: Option<u64>,
    attempts: Option<u8>,
) -> AppResult<PingResult> {
    let result = tcp_ping(
        &host,
        port,
        timeout_ms.unwrap_or(1_500),
        attempts.unwrap_or(2),
    )
    .await?;

    state
        .latency
        .record(&LatencyStore::key(&host, port), now_ms(), result.rtt_ms);

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_natively_implemented_protocol_wins() {
        assert_eq!(
            choose(&[QueryProtocol::Frostbite, QueryProtocol::A2S]),
            QueryProtocol::A2S
        );
        assert_eq!(
            choose(&[QueryProtocol::Minecraft, QueryProtocol::Gamespy3]),
            QueryProtocol::Minecraft
        );
    }

    #[test]
    fn an_unimplemented_or_empty_list_falls_back_to_tcp() {
        assert_eq!(choose(&[QueryProtocol::Frostbite]), QueryProtocol::TcpOnly);
        assert_eq!(choose(&[]), QueryProtocol::TcpOnly);
    }
}
