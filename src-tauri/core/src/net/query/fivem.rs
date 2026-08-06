//! FiveM / CitizenFX — GTA V and RedM multiplayer.
//!
//! Not a game protocol at all: a FiveM server runs an HTTP endpoint on the same
//! port as the game, and the browser reads `/info.json` (name, version, vars)
//! and `/dynamic.json` (live counts). `/players.json` is a third call and is
//! only made when a roster was actually asked for, because on a 2048-slot
//! server it is by far the largest of the three.
//!
//! The HTTP is spoken by hand (see [`http_get`]) rather than through `reqwest`:
//! a full client follows redirects, and a server that answers a query with a
//! 302 would otherwise have the app fetching an arbitrary URL of its choosing.

use std::net::SocketAddr;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::net::query::{http_get, PlayerEntry, QueryProtocol, ServerQueryResult};
use crate::net::reader::strip_colour_codes;

const MAX_PLAYERS: usize = 128;

#[derive(Deserialize)]
struct Dynamic {
    #[serde(default)]
    clients: Option<i64>,
    #[serde(default, rename = "sv_maxclients")]
    max_clients: Option<serde_json::Value>,
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    gametype: Option<String>,
    #[serde(default)]
    mapname: Option<String>,
}

#[derive(Deserialize)]
struct Info {
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    vars: Option<std::collections::BTreeMap<String, serde_json::Value>>,
}

#[derive(Deserialize)]
struct Player {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    ping: Option<i64>,
    #[serde(default)]
    id: Option<i64>,
}

pub async fn query(
    addr: SocketAddr,
    timeout: Duration,
    want_players: bool,
) -> AppResult<ServerQueryResult> {
    let host_header = addr.to_string();

    let (dynamic_raw, rtt) = http_get(addr, &host_header, "/dynamic.json", timeout).await?;

    let dynamic: Dynamic = serde_json::from_slice(&dynamic_raw)
        .map_err(|_| AppError::invalid("This does not look like a FiveM server."))?;

    let mut out = ServerQueryResult::new(QueryProtocol::Fivem, addr.port(), rtt);

    out.name = dynamic.hostname.as_deref().map(strip_fivem_colours);
    out.game = dynamic.gametype;
    out.map = dynamic.mapname;
    out.players = dynamic.clients.and_then(|n| u32::try_from(n.max(0)).ok());

    /*
     * `sv_maxclients` is a string on some builds and a number on others —
     * FiveM's own var system stringifies everything, but `/dynamic.json`
     * sometimes bypasses it. Accepting both is cheaper than being wrong on half
     * the servers.
     */
    out.max_players = dynamic.max_clients.as_ref().and_then(json_to_u32);

    // Best-effort: a server can serve /dynamic.json and 404 /info.json.
    if let Ok((info_raw, _)) = http_get(addr, &host_header, "/info.json", timeout).await {
        if let Ok(info) = serde_json::from_slice::<Info>(&info_raw) {
            out.version = info.server;

            if let Some(vars) = info.vars {
                if out.max_players.is_none() {
                    out.max_players = vars.get("sv_maxClients").and_then(json_to_u32);
                }

                /*
                 * No password field. FiveM has no server password concept in
                 * `/info.json` — access control is a resource's business, not
                 * the platform's — so this stays `None` ("not reported") rather
                 * than being inferred from an unrelated var. Rendering a
                 * guessed "Password: No" is worse than rendering nothing.
                 */
                if let Some(lan) = vars.get("sv_lan").and_then(json_to_string) {
                    out.rules.insert("sv_lan".into(), lan);
                }

                for key in ["locale", "tags", "sv_projectName", "sv_projectDesc"] {
                    if let Some(value) = vars.get(key).and_then(json_to_string) {
                        out.rules.insert(key.to_string(), value);
                    }
                }
            }
        }
    }

    if want_players {
        if let Ok((players_raw, _)) = http_get(addr, &host_header, "/players.json", timeout).await {
            if let Ok(players) = serde_json::from_slice::<Vec<Player>>(&players_raw) {
                out.player_list = players
                    .into_iter()
                    .take(MAX_PLAYERS)
                    .filter_map(|p| {
                        let name = strip_fivem_colours(p.name.as_deref()?);

                        (!name.is_empty()).then_some(PlayerEntry {
                            name,
                            score: p.id.and_then(|v| i32::try_from(v).ok()),
                            duration: None,
                            ping: p
                                .ping
                                .filter(|v| *v >= 0)
                                .and_then(|v| u32::try_from(v).ok()),
                        })
                    })
                    .collect();
            }
        }
    }

    Ok(out)
}

fn json_to_u32(value: &serde_json::Value) -> Option<u32> {
    match value {
        serde_json::Value::Number(n) => n.as_i64().and_then(|v| u32::try_from(v.max(0)).ok()),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn json_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.chars().take(256).collect()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// FiveM uses `^0`–`^9` for colour, the same as Quake. It also permits `~r~`
/// style GTA markup in some fields.
fn strip_fivem_colours(raw: &str) -> String {
    let without_gta = {
        let mut out = String::with_capacity(raw.len());
        let mut chars = raw.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '~' {
                // `~x~` — consume up to the closing tilde, bounded so an
                // unterminated marker cannot eat the rest of the name.
                let mut consumed = 0;

                for next in chars.by_ref() {
                    consumed += 1;

                    if next == '~' || consumed > 4 {
                        break;
                    }
                }

                continue;
            }

            out.push(c);
        }

        out
    };

    strip_colour_codes(&without_gta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_numeric_or_string_max_clients() {
        assert_eq!(json_to_u32(&serde_json::json!(64)), Some(64));
        assert_eq!(json_to_u32(&serde_json::json!("64")), Some(64));
        assert_eq!(json_to_u32(&serde_json::json!(-1)), Some(0));
        assert_eq!(json_to_u32(&serde_json::json!(true)), None);
        assert_eq!(json_to_u32(&serde_json::json!("many")), None);
    }

    #[test]
    fn strips_both_colour_dialects() {
        assert_eq!(strip_fivem_colours("^1Red ^7Server"), "Red Server");
        assert_eq!(strip_fivem_colours("~r~Danger~s~ Zone"), "Danger Zone");
    }

    #[test]
    fn an_unterminated_gta_marker_does_not_eat_the_name() {
        // Without the bound, `~` would swallow everything after it.
        let out = strip_fivem_colours("~rrrrrrrrrrServer Name");

        assert!(out.contains("Server Name") || out.contains("rServer Name"));
    }

    #[test]
    fn parses_a_dynamic_payload() {
        let raw = br#"{"clients":18,"sv_maxclients":"48","hostname":"^2My ^7Server","gametype":"Roleplay","mapname":"fivem-map-skater"}"#;

        let dynamic: Dynamic = serde_json::from_slice(raw).expect("parses");

        assert_eq!(dynamic.clients, Some(18));
        assert_eq!(dynamic.max_clients.as_ref().and_then(json_to_u32), Some(48));
        assert_eq!(
            strip_fivem_colours(dynamic.hostname.as_deref().expect("hostname")),
            "My Server"
        );
    }

    #[test]
    fn a_players_payload_with_missing_fields_still_parses() {
        let raw = br#"[{"name":"^3alice","ping":42,"id":1},{"ping":-1},{"name":"","id":3}]"#;

        let players: Vec<Player> = serde_json::from_slice(raw).expect("parses");

        let mapped: Vec<PlayerEntry> = players
            .into_iter()
            .filter_map(|p| {
                let name = strip_fivem_colours(p.name.as_deref()?);

                (!name.is_empty()).then_some(PlayerEntry {
                    name,
                    score: p.id.and_then(|v| i32::try_from(v).ok()),
                    duration: None,
                    ping: p
                        .ping
                        .filter(|v| *v >= 0)
                        .and_then(|v| u32::try_from(v).ok()),
                })
            })
            .collect();

        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].name, "alice");
        assert_eq!(mapped[0].ping, Some(42));
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        assert!(serde_json::from_slice::<Dynamic>(b"not json").is_err());
        assert!(serde_json::from_slice::<Vec<Player>>(b"{}").is_err());
    }
}
