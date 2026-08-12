//! Live server queries, spoken natively.
//!
//! The website already knows which protocol each game speaks — `App.
//! srvQueryProtocols`, the same list its own scanners use — so the app asks the
//! API for the protocol and then talks to the server **directly**. That is the
//! difference between the browser's numbers, which are as fresh as the last
//! scan, and the app's, which are live and measured from the user's own
//! connection. A player in Sydney and one in Frankfurt see different latencies
//! to the same box, and only the app can tell either of them the truth.
//!
//! Protocols not implemented natively fall back to [`QueryProtocol::TcpOnly`],
//! which still yields a real latency figure, and can be covered by a Server
//! Live Query plugin (`crate::plugins::query`) without an app update.

pub mod a2s;
pub mod fivem;
pub mod gamespy;
pub mod minecraft;
pub mod quake3;
pub mod samp;

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::net::addr::resolve_public;
use crate::net::transport::{clamp_timeout, tcp_exchange, MAX_TIMEOUT_MS};

/// Mirrors website-city's `SpyQueryProtocols`, plus a fallback.
///
/// Deliberately the same names: the value arrives over the API straight out of
/// that enum, and a translation table between the two would be one more place
/// for a game to end up querying with the wrong protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QueryProtocol {
    /// Valve's A2S — Source and GoldSrc. CS2, TF2, Rust, ARK, Garry's Mod…
    ///
    /// **The explicit rename is load-bearing.** `SCREAMING_SNAKE_CASE` splits on
    /// the boundary between a digit and a following letter, so serde derives
    /// `A2_S` from this variant — a name nothing on the wire ever uses. Every
    /// A2S row then failed to deserialise, and because the command takes a
    /// `Vec<QueryRequest>`, ONE such row rejected the whole batch: an entire
    /// screenful of servers showed as timed out because one of them ran a
    /// Source game. It is the only variant the derive gets wrong, which is
    /// exactly why it survived a reading of the enum.
    #[serde(rename = "A2S")]
    A2S,
    /// Modern Minecraft: handshake + status over TCP.
    Minecraft,
    /// Legacy Minecraft Server List Ping (`0xFE 0x01`), pre-1.7 servers.
    MinecraftSlp,
    /// `getstatus` — Quake 3 and its descendants (CoD, Wolfenstein, Xonotic).
    Quake3,
    /// GameSpy v1: plain `\status\`.
    Gamespy1,
    /// GameSpy v2: binary, with a per-section request byte.
    Gamespy2,
    /// GameSpy v3: challenge/response, used by Minecraft's query port too.
    Gamespy3,
    /// San Andreas Multiplayer.
    Samp,
    /// FiveM / CitizenFX — HTTP JSON endpoints rather than a game protocol.
    Fivem,

    // ---- Recognised but not natively implemented; see `is_native`. ----
    Gamespy4,
    Discord,
    Teamspeak3,
    HytaleNitrado,
    Frostbite,
    GtaNetwork,
    GtaRage,
    Scum,

    /// No protocol: measure the TCP handshake and report nothing else.
    TcpOnly,
}

impl QueryProtocol {
    /// Whether this crate can actually speak it.
    ///
    /// The unimplemented variants are still carried rather than dropped so the
    /// UI can say "this game's protocol needs a plugin" instead of silently
    /// showing a latency-only row and looking broken.
    pub fn is_native(self) -> bool {
        matches!(
            self,
            Self::A2S
                | Self::Minecraft
                | Self::MinecraftSlp
                | Self::Quake3
                | Self::Gamespy1
                | Self::Gamespy2
                | Self::Gamespy3
                | Self::Samp
                | Self::Fivem
                | Self::TcpOnly
        )
    }

    /// The transport, for the port-resolution rules in [`QueryTarget`].
    pub fn is_tcp(self) -> bool {
        matches!(
            self,
            Self::Minecraft | Self::MinecraftSlp | Self::Fivem | Self::TcpOnly
        )
    }
}

/// One player, when the protocol reports a roster.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerEntry {
    pub name: String,
    pub score: Option<i32>,
    /// Seconds connected, when reported.
    pub duration: Option<u32>,
    pub ping: Option<u32>,
}

/// What every protocol resolves to.
///
/// One shape for nine wire formats, so the UI has a single row type and adding
/// a tenth protocol changes nothing above this module. Fields a given protocol
/// cannot report are `None` rather than zero — "0 players" and "this protocol
/// does not say" are different facts, and rendering the second as the first is
/// how a server browser earns a reputation for lying.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerQueryResult {
    pub online: bool,
    /// Round-trip time in milliseconds, measured on this exchange.
    pub rtt_ms: u32,
    pub protocol: QueryProtocol,

    pub name: Option<String>,
    pub map: Option<String>,
    pub game: Option<String>,
    pub version: Option<String>,

    pub players: Option<u32>,
    pub max_players: Option<u32>,
    pub bots: Option<u32>,

    pub password: Option<bool>,
    pub secure: Option<bool>,

    /// Populated only when the caller asked for it and the protocol supports it.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub player_list: Vec<PlayerEntry>,

    /// Protocol-specific extras (`sv_gravity`, gamemode, tags…).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty", default)]
    pub rules: std::collections::BTreeMap<String, String>,

    /// The port actually queried, which is often not the game port.
    pub queried_port: u16,
}

impl ServerQueryResult {
    pub fn offline(protocol: QueryProtocol, port: u16) -> Self {
        Self {
            online: false,
            rtt_ms: 0,
            protocol,
            name: None,
            map: None,
            game: None,
            version: None,
            players: None,
            max_players: None,
            bots: None,
            password: None,
            secure: None,
            player_list: Vec::new(),
            rules: Default::default(),
            queried_port: port,
        }
    }

    pub fn new(protocol: QueryProtocol, port: u16, rtt_ms: u32) -> Self {
        Self {
            online: true,
            rtt_ms,
            ..Self::offline(protocol, port)
        }
    }
}

/// Everything needed to query one server.
///
/// The port rules mirror website-city's own scanner config (`App.
/// srvSwapGamePort`, `App.srvGamePortQueryOffset`) so the app and the site
/// agree about which port to hit. Getting this wrong is the single most common
/// reason a server browser shows a live server as dead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryTarget {
    pub host: String,
    /// The port players connect on.
    pub game_port: u16,
    /// An explicit query port from the server's own record, when it has one.
    #[serde(default)]
    pub query_port: Option<u16>,
    /// `App.srvGamePortQueryOffset` — added to the game port when there is no
    /// explicit query port.
    #[serde(default)]
    pub port_offset: i32,
    /// `App.srvSwapGamePort` — query the game port itself, ignoring the offset.
    #[serde(default)]
    pub swap_game_port: bool,
    #[serde(default)]
    pub protocol: Option<QueryProtocol>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Ask for the player roster too. Off by default: it is a second round trip
    /// per server, which a grid of fifty does not want.
    #[serde(default)]
    pub want_players: bool,
}

impl QueryTarget {
    pub fn protocol(&self) -> QueryProtocol {
        self.protocol.unwrap_or(QueryProtocol::TcpOnly)
    }

    pub fn timeout(&self) -> Duration {
        clamp_timeout(self.timeout_ms.unwrap_or(1_500))
    }

    /// Resolve the port to actually query.
    ///
    /// THE RULE, AND WHERE IT COMES FROM
    /// ---------------------------------
    /// An explicit `query_port` wins; otherwise the GAME port is queried. That
    /// is it. It is copied from the scanner that populates these rows —
    /// `spy/internal/scanners/server.go`:
    ///
    /// ```text
    /// qPort := port
    /// if srv.PortQuery != nil && *srv.PortQuery > 0 {
    ///     qPort = *srv.PortQuery
    /// }
    /// ```
    ///
    /// **`port_offset` is deliberately NOT applied**, and that is the fix for
    /// "Source servers never answer". `App.srvGamePortQueryOffset` reaches the
    /// app as `portOffset`, but grepping the scanner for it finds exactly one
    /// hit — the struct field it is deserialised into. Nothing ever reads it.
    /// So a game whose App row carries a stale non-zero offset is scanned by the
    /// site on its game port and was probed by this app on `game + offset`,
    /// which for a UDP game protocol is silence and renders as a timeout.
    ///
    /// **`swap_game_port` is not a port-selection input either.** In the scanner
    /// (`spy/internal/protocols/a2s.go`) it means "after A2S_INFO comes back,
    /// read the true game port out of the extended-info block and swap the
    /// stored `port`/`portQuery`". It is post-scan bookkeeping about which port
    /// is the GAME port; reading it as "query the game port" happened to give
    /// the right number about half the time, which is worse than being wrong
    /// consistently.
    ///
    /// Both fields stay on [`QueryTarget`] because the contract sends them and
    /// dropping them would be a wire change. They are simply not consulted here.
    ///
    /// The per-protocol exceptions below are the scanner's own hardcoded ones,
    /// and they apply only when the row carries no explicit query port.
    pub fn resolve_port(&self) -> AppResult<u16> {
        if let Some(port) = self.query_port.filter(|p| *p > 0) {
            return Ok(port);
        }

        if self.game_port == 0 {
            return Err(AppError::invalid("This server has no port to query."));
        }

        Ok(match self.protocol() {
            // `spy/internal/protocols/fivem.go`: the CitizenFX HTTP endpoint is
            // on 30120 unless the row says otherwise.
            QueryProtocol::Fivem => 30120,
            // `spy/internal/protocols/scum.go`: game port + 2.
            QueryProtocol::Scum => self.game_port.saturating_add(2),
            _ => self.game_port,
        })
    }
}

/// Query one server with whichever protocol its game speaks.
pub async fn query(target: &QueryTarget) -> AppResult<ServerQueryResult> {
    let protocol = target.protocol();
    let port = target.resolve_port()?;
    let timeout = target.timeout();

    /*
     * Resolved ONCE, here, and the `SocketAddr` is what every protocol module
     * connects to. None of them take a hostname, so none of them can be made to
     * re-resolve to an address the guard already rejected.
     */
    let addr = resolve_public(&target.host, port).await?;

    match protocol {
        QueryProtocol::A2S => a2s::query(addr, timeout, target.want_players).await,
        QueryProtocol::Minecraft => minecraft::query(addr, &target.host, timeout).await,
        QueryProtocol::MinecraftSlp => minecraft::query_legacy(addr, timeout).await,
        QueryProtocol::Quake3 => quake3::query(addr, timeout).await,
        QueryProtocol::Gamespy1 => gamespy::query_v1(addr, timeout).await,
        QueryProtocol::Gamespy2 => gamespy::query_v2(addr, timeout, target.want_players).await,
        QueryProtocol::Gamespy3 => gamespy::query_v3(addr, timeout, target.want_players).await,
        QueryProtocol::Samp => samp::query(addr, timeout).await,
        QueryProtocol::Fivem => fivem::query(addr, timeout, target.want_players).await,

        // Everything else, including the recognised-but-unimplemented set.
        _ => tcp_only(addr, timeout, protocol).await,
    }
    .map(|mut result| {
        result.queried_port = port;
        result
    })
}

/// The fallback: measure the TCP handshake and report nothing else.
///
/// Still worth doing. Latency is the number the app exists to provide, and a
/// completed handshake is real evidence the server is up — more than the
/// website's last scan can offer for a box that went down a minute ago.
async fn tcp_only(
    addr: std::net::SocketAddr,
    timeout: Duration,
    protocol: QueryProtocol,
) -> AppResult<ServerQueryResult> {
    use std::time::Instant;
    use tokio::net::TcpStream;

    let started = Instant::now();

    let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| AppError::Network("The server did not answer.".into()))??;

    let rtt = crate::net::transport::elapsed_ms(started);

    // Closed immediately: an open socket shows up as a phantom player slot on
    // several of these games.
    drop(stream);

    Ok(ServerQueryResult::new(protocol, addr.port(), rtt))
}

/// A raw HTTP GET, for the protocols that are really web APIs.
///
/// Bounded by the same deadline as everything else, and it speaks HTTP/1.0 by
/// hand rather than pulling `reqwest` in: these endpoints are plain, and the
/// full client would follow redirects to wherever a hostile server points.
pub(crate) async fn http_get(
    addr: std::net::SocketAddr,
    host_header: &str,
    path: &str,
    timeout: Duration,
) -> AppResult<(Vec<u8>, u32)> {
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: TMC-App\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );

    let (raw, rtt) = tcp_exchange(addr, request.as_bytes(), timeout).await?;

    // Split the head from the body. A reply with no blank line is not HTTP.
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| AppError::invalid("The server did not send an HTTP reply."))?;

    let body = raw.get(split + 4..).unwrap_or(&[]).to_vec();

    Ok((body, rtt))
}

/// Shared ceiling so a caller cannot ask for an unbounded wait.
pub const QUERY_MAX_TIMEOUT_MS: u64 = MAX_TIMEOUT_MS;

#[cfg(test)]
mod tests {
    use super::*;

    fn target(query_port: Option<u16>, offset: i32, swap: bool) -> QueryTarget {
        QueryTarget {
            host: "example.test".into(),
            game_port: 27015,
            query_port,
            port_offset: offset,
            swap_game_port: swap,
            protocol: Some(QueryProtocol::A2S),
            timeout_ms: None,
            want_players: false,
        }
    }

    #[test]
    fn an_explicit_query_port_wins() {
        assert_eq!(
            target(Some(27016), 5, true).resolve_port().expect("port"),
            27016
        );
    }

    /// The regression this rule exists for. `App.srvGamePortQueryOffset` is
    /// dead config in the scanner — one hit, the struct field, never read — so
    /// applying it here probed a port the site never scans. On a UDP game
    /// protocol that is silence, and every Source server rendered as a timeout.
    #[test]
    fn the_offset_is_never_applied() {
        for offset in [-30000, -1, 1, 5, 60000] {
            assert_eq!(
                target(None, offset, false).resolve_port().expect("port"),
                27015,
                "offset {offset} must not move the query port"
            );
        }
    }

    /// `swapGamePort` is post-scan bookkeeping in the scanner, not a port
    /// rule. Either way the answer is the game port, and it must be the game
    /// port for the same reason in both cases.
    #[test]
    fn swap_game_port_does_not_change_the_answer() {
        assert_eq!(target(None, 7, true).resolve_port().expect("port"), 27015);
        assert_eq!(target(None, 7, false).resolve_port().expect("port"), 27015);
    }

    #[test]
    fn the_scanners_per_protocol_defaults_are_matched() {
        let mut t = target(None, 0, false);

        t.protocol = Some(QueryProtocol::Fivem);
        assert_eq!(t.resolve_port().expect("port"), 30120);

        t.protocol = Some(QueryProtocol::Scum);
        assert_eq!(t.resolve_port().expect("port"), 27017);

        // …and an explicit query port still overrides them.
        t.query_port = Some(40120);
        assert_eq!(t.resolve_port().expect("port"), 40120);
    }

    #[test]
    fn a_zero_port_is_refused() {
        let mut t = target(None, 0, false);
        t.game_port = 0;

        assert!(t.resolve_port().is_err());

        // A zero explicit query port falls through to the game port rather than
        // being used — a server row with `portQuery: 0` means "not set".
        let mut t = target(Some(0), 0, false);
        t.game_port = 27015;

        assert_eq!(t.resolve_port().expect("port"), 27015);
    }

    #[test]
    fn timeouts_are_clamped() {
        let mut t = target(None, 0, false);
        t.timeout_ms = Some(u64::MAX);

        assert!(t.timeout() <= Duration::from_millis(MAX_TIMEOUT_MS));

        t.timeout_ms = Some(0);
        assert!(t.timeout() >= Duration::from_millis(100));
    }

    /// Every protocol name, exactly as `SpyQueryProtocols` spells it on the
    /// wire, plus this crate's own fallback.
    ///
    /// Hardcoded rather than derived from the enum: deriving it from the same
    /// `Serialize` impl under test would agree with any rename, correct or not,
    /// which is precisely the bug this exists to catch.
    const WIRE_NAMES: &[(&str, QueryProtocol)] = &[
        ("A2S", QueryProtocol::A2S),
        ("MINECRAFT", QueryProtocol::Minecraft),
        ("MINECRAFT_SLP", QueryProtocol::MinecraftSlp),
        ("QUAKE3", QueryProtocol::Quake3),
        ("DISCORD", QueryProtocol::Discord),
        ("TEAMSPEAK3", QueryProtocol::Teamspeak3),
        ("HYTALE_NITRADO", QueryProtocol::HytaleNitrado),
        ("FIVEM", QueryProtocol::Fivem),
        ("FROSTBITE", QueryProtocol::Frostbite),
        ("GAMESPY1", QueryProtocol::Gamespy1),
        ("GAMESPY2", QueryProtocol::Gamespy2),
        ("GAMESPY3", QueryProtocol::Gamespy3),
        ("GAMESPY4", QueryProtocol::Gamespy4),
        ("GTA_NETWORK", QueryProtocol::GtaNetwork),
        ("GTA_RAGE", QueryProtocol::GtaRage),
        ("SAMP", QueryProtocol::Samp),
        ("SCUM", QueryProtocol::Scum),
        ("TCP_ONLY", QueryProtocol::TcpOnly),
    ];

    /// The regression that took every Source server offline.
    ///
    /// `SCREAMING_SNAKE_CASE` derives `A2_S` from the `A2S` variant, because it
    /// splits between a digit and the letter after it. Nothing on the wire uses
    /// that name, so every A2S row failed to deserialise — and one bad row in a
    /// `Vec<QueryRequest>` fails the whole batch.
    #[test]
    fn every_protocol_round_trips_under_its_wire_name() {
        for (name, protocol) in WIRE_NAMES {
            let json = format!("\"{name}\"");

            let parsed: QueryProtocol = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("`{name}` must deserialise: {e}"));

            assert_eq!(parsed, *protocol, "`{name}` parsed to the wrong variant");

            assert_eq!(
                serde_json::to_string(protocol).expect("serialise"),
                json,
                "`{protocol:?}` must serialise back as `{name}`"
            );
        }
    }

    /// One A2S row must not be able to take the rest of a batch down with it.
    #[test]
    fn a_batch_of_mixed_protocols_deserialises_whole() {
        let raw = r#"["A2S","MINECRAFT","A2S","TCP_ONLY"]"#;

        let parsed: Vec<QueryProtocol> = serde_json::from_str(raw).expect("batch");

        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0], QueryProtocol::A2S);
    }

    #[test]
    fn every_variant_answers_is_native_without_panicking() {
        for p in [
            QueryProtocol::A2S,
            QueryProtocol::Minecraft,
            QueryProtocol::Frostbite,
            QueryProtocol::TcpOnly,
        ] {
            let _ = p.is_native();
            let _ = p.is_tcp();
        }
    }
}
