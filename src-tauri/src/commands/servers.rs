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
use tmc_core::plugins::manifest::ServerQuery;

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
    /// The app this server belongs to, so a Server Live Query PLUGIN can be
    /// matched to it when no built-in protocol applies. Optional because the
    /// view page's single-server call does not always have one to hand.
    #[serde(default)]
    pub app_id: Option<i64>,
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

/// The Server Live Query plugin covering an app, if the user has approved one.
///
/// This is what makes the browser work for EVERY game rather than only the nine
/// with a built-in protocol. A game whose protocol this build does not speak
/// falls back to `TCP_ONLY` — still a real latency figure, but no player count —
/// and a plugin closes that gap without an app update.
///
/// Resolved per batch, not per server: the registry lock is taken once for a
/// screenful rather than fifty times.
fn plugin_query_for(state: &AppState, app_id: Option<i64>) -> Option<(String, ServerQuery)> {
    let app_id = app_id?;

    state.plugins.list().into_iter().find_map(|record| {
        if !record.enabled || record.needs_reapproval {
            return None;
        }

        if !record.kinds.iter().any(|k| k == "serverQuery") {
            return None;
        }

        // An empty `apps` means "any game", which is what a generic protocol
        // plugin (GameSpy, say) legitimately declares.
        if !record.apps.is_empty() && !record.apps.contains(&app_id) {
            return None;
        }

        // `active` re-checks the approval fingerprint, which is the gate in
        // front of anything a plugin does — `list` alone is only an index.
        let manifest = state.plugins.active(&record.id).ok()?;
        let spec = manifest.server_query.clone()?;

        Some((record.id, spec))
    })
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
            Some(request) => {
                let plugin = plugin_query_for(&state, request.app_id);

                running.push(run_one(request, plugin));
            }
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
            let plugin = plugin_query_for(&state, request.app_id);

            running.push(run_one(request, plugin));
        }
    }

    Ok(out)
}

async fn run_one(request: QueryRequest, plugin: Option<(String, ServerQuery)>) -> QueryOutcome {
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

    /*
     * A plugin is tried FIRST, and only when the built-in answer would have
     * been `TcpOnly` — i.e. when this build speaks nothing the game declares.
     * Preferring a plugin over a native protocol would mean a plugin could
     * quietly replace a parser that has been fuzzed and tested; preferring it
     * over "no protocol at all" is pure gain.
     *
     * A plugin failure falls THROUGH to the built-in path rather than being
     * reported: the TCP ping still yields a real latency, and a broken plugin
     * should not make a server look offline.
     */
    if protocol == QueryProtocol::TcpOnly {
        if let Some((id, spec)) = plugin {
            match tmc_core::plugins::query::run(
                &spec,
                &request.host,
                request.port,
                request.query_port,
            )
            .await
            {
                Ok(fields) => {
                    return QueryOutcome {
                        id: request.id,
                        key,
                        result: Some(from_plugin_fields(&fields, request.port)),
                        error: None,
                    }
                }
                Err(err) => tracing::debug!(
                    plugin = %id,
                    "query plugin failed, falling back to TCP: {}",
                    err.detail()
                ),
            }
        }
    }

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

/// Map a query plugin's field bag onto the shape every card already renders.
///
/// The plugin names its own fields, so this looks for the conventional ones and
/// puts everything else in `rules`. Nothing is required: a plugin that only
/// reports a player count still produces a usable row, which is the point of
/// letting a game be covered by data rather than by code.
fn from_plugin_fields(
    fields: &serde_json::Map<String, serde_json::Value>,
    game_port: u16,
) -> ServerQueryResult {
    let string = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|k| {
            fields
                .get(*k)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .filter(|s| !s.is_empty())
        })
    };

    let number = |keys: &[&str]| -> Option<u32> {
        keys.iter().find_map(|k| {
            fields
                .get(*k)
                .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
                .map(|n| n.min(u64::from(u32::MAX)) as u32)
        })
    };

    let rtt = number(&["_rttMs"]).unwrap_or(0);
    let port = number(&["_port"])
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(game_port);

    let mut rules = std::collections::BTreeMap::new();

    for (key, value) in fields {
        // The two underscore-prefixed keys are ours, not the plugin's.
        if key.starts_with('_') {
            continue;
        }

        if let Some(text) = value.as_str() {
            rules.insert(key.clone(), text.to_string());
        } else if !value.is_null() {
            rules.insert(key.clone(), value.to_string());
        }
    }

    ServerQueryResult {
        online: true,
        rtt_ms: rtt,
        protocol: QueryProtocol::TcpOnly,
        name: string(&["name", "hostname", "serverName"]),
        map: string(&["map", "mapName"]),
        game: string(&["game", "gameMode", "gametype"]),
        version: string(&["version"]),
        players: number(&["players", "playerCount", "numPlayers", "curUsers"]),
        max_players: number(&["maxPlayers", "maxplayers", "maxUsers"]),
        bots: number(&["bots"]),
        password: fields.get("password").and_then(|v| v.as_bool()),
        secure: fields.get("secure").and_then(|v| v.as_bool()),
        player_list: Vec::new(),
        rules,
        queried_port: port,
    }
}

/// One server, in full — the view page's live panel, with the roster.
#[tauri::command]
pub async fn query_server(
    state: State<'_, AppState>,
    request: QueryRequest,
) -> AppResult<QueryOutcome> {
    let plugin = plugin_query_for(&state, request.app_id);

    let outcome = run_one(request, plugin).await;

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

    /// Stand-in for "recognised, but this crate cannot speak it".
    ///
    /// It has to be a variant `is_native` rejects, so implementing a protocol
    /// natively breaks these two tests — which is the intended signal, not a
    /// nuisance: pick the next one still on the list. Frostbite and TeamSpeak 3
    /// both used to sit here and neither qualifies any more.
    ///
    /// Still on the list: `Discord`, `GtaNetwork`, `GtaRage`, `Scum` — and all
    /// four are there for a REASON rather than for want of writing, which is
    /// the thing to read before picking one off it. Each is a protocol the
    /// scanner speaks by asking a third party rather than the server, so doing
    /// it from a user's device would measure that third party's hosting and
    /// tell it every server the user scrolled past. `net::query::hytale`'s
    /// module header spells this out.
    const UNIMPLEMENTED: QueryProtocol = QueryProtocol::Scum;

    #[test]
    fn the_stand_in_is_still_unimplemented() {
        assert!(!UNIMPLEMENTED.is_native());
    }

    #[test]
    fn the_first_natively_implemented_protocol_wins() {
        assert_eq!(
            choose(&[UNIMPLEMENTED, QueryProtocol::A2S]),
            QueryProtocol::A2S
        );
        assert_eq!(
            choose(&[QueryProtocol::Minecraft, QueryProtocol::Gamespy3]),
            QueryProtocol::Minecraft
        );
    }

    #[test]
    fn an_unimplemented_or_empty_list_falls_back_to_tcp() {
        assert_eq!(choose(&[UNIMPLEMENTED]), QueryProtocol::TcpOnly);
        assert_eq!(choose(&[]), QueryProtocol::TcpOnly);
    }
}
