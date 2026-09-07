//! Hytale servers hosted by Nitrado.
//!
//! Not a game protocol: a Nitrado-hosted Hytale server runs an HTTPS endpoint
//! that answers `GET /Nitrado/Query` with one JSON document describing the
//! server, its universe and everyone on it. `spy/internal/protocols/
//! hytale_nitrado.go` is the authority, and this reads the same fields.
//!
//! Because the whole answer arrives in one document, the roster costs nothing
//! extra — unlike FiveM, where `/players.json` is a third request and by far
//! the largest. `want_players` therefore decides whether the names are RETURNED,
//! not whether they are fetched.
//!
//! WHY THIS ONE AND NOT THE OTHER FOUR
//! ----------------------------------
//! Of the protocols the scanner speaks that this crate did not, this is the
//! only one that talks to the SERVER. `SCUM` goes to `api.hellbz.de`,
//! `GTA_NETWORK` to `multiplayerhosting.info`, `GTA_RAGE` to the RAGE:MP master
//! list at `cdn.rage.mp`, and `DISCORD` is a stub that queries nothing at all.
//!
//! Those four cannot be done here, and the reason is the reason this feature
//! exists at all: the app measures latency from the USER's device to the game
//! server. A round trip to a third-party API is the same number for every
//! server in the list and describes that API's hosting rather than the game's —
//! `spy` says so itself in three separate comments, and it does not record a
//! latency for any of them. Doing it from the app would also send every server
//! a user scrolls past to a third party, and on a phone it would be one HTTPS
//! request per row per tick. They stay on `TCP_ONLY`, which measures a real
//! handshake to the real box; the player counts still arrive from the API,
//! which got them from the scanner, which read those lists once for everybody.
//!
//! THE CERTIFICATE
//! ---------------
//! Nitrado's per-server endpoints present self-signed certificates, so the
//! scanner sets `InsecureSkipVerify` and this sets `danger_accept_invalid_certs`
//! for the same reason: the alternative is that the protocol never works for
//! anybody. What that costs is bounded and worth stating plainly — an attacker
//! positioned to intercept the connection can lie about a player count. There
//! is no credential on this request and no secret in the reply, the address
//! already went through [`crate::net::addr::resolve_public`], and the client
//! built here is used for nothing else, so the exemption cannot leak into a
//! request that does carry a bearer.

use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::net::query::{PlayerEntry, QueryProtocol, ServerQueryResult};

/// Query port when the row has no explicit one: game port + 3.
///
/// `NITRADO_PORT_OFFSET` in `spy/internal/protocols/hytale_nitrado.go`.
pub const PORT_OFFSET: u16 = 3;

/// Cap on the roster this returns. A Hytale server is not a 2048-slot FiveM
/// box, but the number comes from the wire and so is bounded like every other.
const MAX_PLAYERS: usize = 512;

/// Cap on the reply. The document holds a plugin table and a player list, both
/// server-controlled.
const MAX_BODY: usize = 1024 * 1024;

#[derive(Deserialize, Default)]
#[serde(default)]
struct Reply {
    #[serde(rename = "Server")]
    server: ServerInfo,
    #[serde(rename = "Universe")]
    universe: UniverseInfo,
    #[serde(rename = "Players")]
    players: Vec<Player>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ServerInfo {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Version")]
    version: Option<String>,
    #[serde(rename = "Patchline")]
    patchline: Option<String>,
    #[serde(rename = "MaxPlayers")]
    max_players: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UniverseInfo {
    #[serde(rename = "CurrentPlayers")]
    current_players: Option<i64>,
    #[serde(rename = "DefaultWorld")]
    default_world: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Player {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "World")]
    world: Option<String>,
}

/// The client this protocol uses, and nothing else does.
///
/// Built once — a TLS client per query would pay a fresh configuration build on
/// every tick of a browser full of servers — and deliberately not shared with
/// [`crate::api::ApiClient`], which attaches a bearer to what it sends.
fn client() -> AppResult<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();

    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                // See the module header. Scoped to this client, which is used
                // only for this one unauthenticated GET.
                .danger_accept_invalid_certs(true)
                /*
                 * No redirects. A server answering a query with a 302 would
                 * otherwise have the app fetching a URL of the server's
                 * choosing — the same reason `fivem` speaks HTTP by hand.
                 */
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .ok()
        })
        .as_ref()
        .ok_or_else(|| AppError::internal("could not build the query HTTP client"))
}

pub async fn query(
    addr: SocketAddr,
    timeout: Duration,
    want_players: bool,
) -> AppResult<ServerQueryResult> {
    /*
     * The URL names the resolved ADDRESS, not the hostname the caller started
     * with. Everything under `net/query` resolves once and connects to that;
     * handing `reqwest` a name here would let it resolve again, which is the
     * DNS-rebinding hole the guard exists to close.
     */
    let url = format!("https://{addr}/Nitrado/Query");

    let started = Instant::now();

    let response = client()?
        .get(&url)
        .header("Accept", "application/json")
        .timeout(timeout)
        .send()
        .await
        .map_err(|e| AppError::Network(format!("The server did not answer: {e}")))?;

    let rtt_ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;

    if !response.status().is_success() {
        return Err(AppError::invalid(format!(
            "The server answered {} rather than a status document.",
            response.status().as_u16()
        )));
    }

    let body = response
        .bytes()
        .await
        .map_err(|e| AppError::Network(format!("The reply was cut short: {e}")))?;

    if body.len() > MAX_BODY {
        return Err(AppError::invalid(
            "The status document is implausibly large.",
        ));
    }

    let reply: Reply = serde_json::from_slice(&body)
        .map_err(|_| AppError::invalid("The server's reply was not a Nitrado status document."))?;

    let mut result = ServerQueryResult::offline(QueryProtocol::HytaleNitrado, addr.port());

    result.online = true;
    result.rtt_ms = rtt_ms;
    result.name = reply.server.name.filter(|n| !n.is_empty());
    result.version = reply.server.version.filter(|v| !v.is_empty());
    result.map = reply.universe.default_world.filter(|w| !w.is_empty());

    result.max_players = clamp_count(reply.server.max_players);
    result.players = clamp_count(reply.universe.current_players);

    if let Some(patchline) = reply.server.patchline.filter(|p| !p.is_empty()) {
        result.rules.insert("patchline".into(), patchline);
    }

    if want_players {
        for player in reply.players.into_iter().take(MAX_PLAYERS) {
            let Some(name) = player.name.filter(|n| !n.is_empty()) else {
                continue;
            };

            result.player_list.push(PlayerEntry {
                name,
                score: None,
                duration: None,
                ping: None,
            });
        }
    }

    /*
     * The measured roster wins over the declared count when there is one, the
     * same rule the rest of this module follows: a number derived from a list
     * that was actually sent is better evidence than a field a server fills in
     * for itself.
     */
    if !result.player_list.is_empty() {
        result.players = Some(result.player_list.len() as u32);
    }

    Ok(result)
}

/// A count from the wire, or `None`.
///
/// Negative is refused rather than saturated to zero: a server reporting `-1`
/// players is reporting that it does not know, and rendering that as `0/40`
/// invents a fact.
fn clamp_count(raw: Option<i64>) -> Option<u32> {
    raw.filter(|n| *n >= 0)
        .map(|n| n.min(u32::MAX as i64) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> Reply {
        serde_json::from_str(body).expect("parse")
    }

    const GOLDEN: &str = r#"{
        "Server": {
            "Name": "Hytale Test",
            "Version": "0.1.4",
            "Revision": "abc123",
            "Patchline": "live",
            "ProtocolVersion": 3,
            "ProtocolHash": "deadbeef",
            "MaxPlayers": 40
        },
        "Universe": { "CurrentPlayers": 2, "DefaultWorld": "Orbis" },
        "Players": [
            { "Name": "alice", "UUID": "a", "World": "Orbis" },
            { "Name": "bob", "UUID": "b", "World": "Orbis" }
        ],
        "Plugins": { "core": { "Version": "1", "Loaded": true, "Enabled": true, "State": "ok" } }
    }"#;

    #[test]
    fn a_golden_reply_reads_as_expected() {
        let reply = parse(GOLDEN);

        assert_eq!(reply.server.name.as_deref(), Some("Hytale Test"));
        assert_eq!(reply.server.max_players, Some(40));
        assert_eq!(reply.universe.current_players, Some(2));
        assert_eq!(reply.universe.default_world.as_deref(), Some("Orbis"));
        assert_eq!(reply.players.len(), 2);
    }

    /// Every field is optional. A server running a build that predates one of
    /// them, or a Nitrado revision that renames one, must produce a row with
    /// less on it rather than an error — the latency is still worth having.
    #[test]
    fn a_reply_missing_everything_is_still_a_reply() {
        let reply = parse("{}");

        assert!(reply.server.name.is_none());
        assert!(reply.players.is_empty());

        let reply = parse(r#"{"Server":{},"Universe":{},"Players":[]}"#);

        assert!(reply.universe.current_players.is_none());
    }

    /// The document is server-controlled, so the shapes it can be wrong in are
    /// the normal case rather than an edge case. None of these may panic.
    #[test]
    fn a_hostile_document_is_an_error_and_never_a_panic() {
        let cases = [
            "",
            "null",
            "[]",
            "\"a string\"",
            r#"{"Server": []}"#,
            r#"{"Server": {"MaxPlayers": "forty"}}"#,
            r#"{"Players": {}}"#,
            r#"{"Players": [null]}"#,
            r#"{"Universe": {"CurrentPlayers": 1e400}}"#,
        ];

        for case in cases {
            // Either shape is acceptable; what is not acceptable is a panic.
            let _ = serde_json::from_str::<Reply>(case);
        }
    }

    /// Every prefix of a valid document, through the parser. The release
    /// profile aborts on panic, so a parser that can panic on truncated input
    /// is a remote kill switch.
    #[test]
    fn no_truncation_of_a_valid_reply_panics() {
        let bytes = GOLDEN.as_bytes();

        for end in 0..=bytes.len() {
            let _ = serde_json::from_slice::<Reply>(&bytes[..end]);
        }
    }

    #[test]
    fn a_negative_count_is_unknown_rather_than_zero() {
        assert_eq!(clamp_count(Some(-1)), None);
        assert_eq!(clamp_count(Some(0)), Some(0));
        assert_eq!(clamp_count(Some(40)), Some(40));
        assert_eq!(clamp_count(None), None);
        // Bigger than a u32 saturates rather than wrapping to something small.
        assert_eq!(clamp_count(Some(i64::MAX)), Some(u32::MAX));
    }
}
