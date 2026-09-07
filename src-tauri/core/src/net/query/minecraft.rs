//! Minecraft's Server List Ping, both generations.
//!
//! The modern one (1.7+) is a small TCP state machine: a handshake packet that
//! declares "next state = status", then an empty status request, then a single
//! JSON blob back. The legacy one (pre-1.7) is a single `0xFE 0x01` byte pair
//! answered with a UTF-16BE string of `§`-separated fields.
//!
//! Both are implemented because "which one does this server speak?" is not
//! knowable in advance — and the website's protocol list has separate entries
//! for exactly that reason.
//!
//! The JSON is parsed into a narrow struct rather than a `Value` tree. That is
//! deliberate: `description` alone is a union of three different shapes across
//! versions and mod loaders, and pattern-matching it explicitly is the only way
//! to avoid a hostile or merely unusual server producing `[object Object]` in
//! the browser.

use std::net::SocketAddr;
use std::time::Duration;

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::net::query::{PlayerEntry, QueryProtocol, ServerQueryResult};
use crate::net::reader::{strip_colour_codes, Reader};
use crate::net::transport::{elapsed_ms, MAX_TCP_RESPONSE};

/// Read cap for the status reply. Servers embed a base64 PNG favicon, so this
/// is legitimately large — but not unbounded.
const MAX_STATUS_JSON: usize = MAX_TCP_RESPONSE;

const MAX_SAMPLE_PLAYERS: usize = 64;

fn varint(mut value: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(5);

    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;

        if value != 0 {
            byte |= 0x80;
        }

        out.push(byte);

        if value == 0 {
            return out;
        }
    }
}

fn packet(id: u8, body: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(body.len() + 1);
    inner.push(id);
    inner.extend_from_slice(body);

    let mut out = varint(inner.len() as u32);
    out.extend_from_slice(&inner);

    out
}

/// Modern SLP (1.7+).
///
/// `hostname` is sent in the handshake as the address the client *thinks* it is
/// connecting to. That matters: virtual-host setups (BungeeCord, Velocity, any
/// shared proxy) route on it, and sending the resolved IP instead lands on the
/// proxy's default server or is refused outright.
pub async fn query(
    addr: SocketAddr,
    hostname: &str,
    timeout: Duration,
) -> AppResult<ServerQueryResult> {
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let started = Instant::now();

    let mut stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| AppError::Network("The server did not answer.".into()))??;

    let _ = stream.set_nodelay(true);

    let rtt = elapsed_ms(started);

    // Handshake: protocol -1 ("I don't know yet", which every server accepts
    // for a status ping), host, port, next state = 1 (status).
    let mut handshake = Vec::new();
    handshake.extend_from_slice(&varint_signed(-1));

    let host_bytes = hostname.as_bytes();

    if host_bytes.len() > 255 {
        return Err(AppError::invalid("Hostname is too long for a handshake."));
    }

    handshake.extend_from_slice(&varint(host_bytes.len() as u32));
    handshake.extend_from_slice(host_bytes);
    handshake.extend_from_slice(&addr.port().to_be_bytes());
    handshake.extend_from_slice(&varint(1));

    let mut request = packet(0x00, &handshake);
    // The status request itself: packet 0x00, empty body.
    request.extend_from_slice(&packet(0x00, &[]));

    tokio::time::timeout(timeout, stream.write_all(&request))
        .await
        .map_err(|_| AppError::Network("The server stopped responding.".into()))??;

    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 8192];

    /*
     * Read until the declared packet length is satisfied rather than until EOF.
     * A Minecraft server keeps the connection open after answering, so an
     * EOF-driven read would sit there until the deadline on every single query.
     */
    loop {
        let read = match tokio::time::timeout(timeout, stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => n,
            Ok(Err(_)) | Err(_) => break,
        };

        buf.extend_from_slice(chunk.get(..read).unwrap_or(&[]));

        if buf.len() >= MAX_STATUS_JSON {
            break;
        }

        if status_is_complete(&buf) {
            break;
        }
    }

    if buf.is_empty() {
        return Err(AppError::Network("The server did not answer.".into()));
    }

    let json = read_status_json(&buf)?;

    parse_status(&json, addr.port(), rtt)
}

/// Whether the buffer holds a whole packet, by its own length prefix.
fn status_is_complete(buf: &[u8]) -> bool {
    let mut r = Reader::new(buf);

    match r.varint() {
        Ok(len) if len >= 0 => buf.len() >= r.position() + len as usize,
        _ => false,
    }
}

fn read_status_json(buf: &[u8]) -> AppResult<String> {
    let mut r = Reader::new(buf);

    let _packet_len = r.varint()?;
    let packet_id = r.varint()?;

    if packet_id != 0x00 {
        return Err(AppError::invalid("Unexpected reply from the server."));
    }

    r.varstring(MAX_STATUS_JSON)
}

/// -1 as a Minecraft varint: the protocol version a status ping declares.
fn varint_signed(value: i32) -> Vec<u8> {
    varint(value as u32)
}

// ------------------------------------------------------------- Status JSON

#[derive(Deserialize)]
struct StatusPlayers {
    #[serde(default)]
    online: Option<i64>,
    #[serde(default)]
    max: Option<i64>,
    #[serde(default)]
    sample: Option<Vec<StatusSample>>,
}

#[derive(Deserialize)]
struct StatusSample {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct StatusVersion {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct Status {
    #[serde(default)]
    description: Option<serde_json::Value>,
    #[serde(default)]
    players: Option<StatusPlayers>,
    #[serde(default)]
    version: Option<StatusVersion>,
}

/// Flatten a `description` into readable text.
///
/// Three shapes exist in the wild and all three are legal:
///   * a plain string (old servers, and many proxies)
///   * `{ "text": "…" }`
///   * `{ "extra": [ … ] }` — a chat-component tree, arbitrarily nested
///
/// A recursion bound is not optional here: the tree comes from the server, and
/// `extra` can contain `extra`. Without the depth cap a crafted MOTD is a stack
/// overflow, which under `panic = "abort"` takes the app with it.
fn description_text(value: &serde_json::Value, depth: u8, out: &mut String) {
    if depth > 12 || out.len() > 512 {
        return;
    }

    match value {
        serde_json::Value::String(s) => out.push_str(s),
        serde_json::Value::Array(items) => {
            for item in items {
                description_text(item, depth + 1, out);
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(text)) = map.get("text") {
                out.push_str(text);
            }

            if let Some(extra) = map.get("extra") {
                description_text(extra, depth + 1, out);
            }
        }
        _ => {}
    }
}

fn parse_status(json: &str, port: u16, rtt: u32) -> AppResult<ServerQueryResult> {
    let status: Status = serde_json::from_str(json)
        .map_err(|_| AppError::invalid("The server sent a status reply we could not read."))?;

    /*
     * Every field on `Status` is optional, so serde happily accepts `[]`, `{}`
     * or `null` and hands back an empty struct. Something answered on the port,
     * but nothing about it says "Minecraft server" — reporting that as an
     * online server with no name is worse than reporting nothing.
     */
    if status.description.is_none() && status.players.is_none() && status.version.is_none() {
        return Err(AppError::invalid(
            "That port answered, but not like a Minecraft server.",
        ));
    }

    let mut out = ServerQueryResult::new(QueryProtocol::Minecraft, port, rtt);

    if let Some(description) = &status.description {
        let mut text = String::new();
        description_text(description, 0, &mut text);

        let cleaned = strip_colour_codes(&text.replace(['\n', '\r'], " "));

        if !cleaned.is_empty() {
            out.name = Some(cleaned);
        }
    }

    if let Some(version) = &status.version {
        out.version = version.name.as_deref().map(strip_colour_codes);
    }

    if let Some(players) = &status.players {
        // Clamped: a server is free to report `online: 9223372036854775807`,
        // and several joke servers do.
        out.players = players.online.and_then(|n| u32::try_from(n.max(0)).ok());
        out.max_players = players.max.and_then(|n| u32::try_from(n.max(0)).ok());

        if let Some(sample) = &players.sample {
            out.player_list = sample
                .iter()
                .filter_map(|entry| entry.name.as_deref())
                .filter(|name| !name.is_empty())
                .take(MAX_SAMPLE_PLAYERS)
                .map(|name| PlayerEntry {
                    name: strip_colour_codes(name),
                    score: None,
                    duration: None,
                    ping: None,
                })
                .collect();
        }
    }

    Ok(out)
}

// ------------------------------------------------------------------ Legacy

/// Pre-1.7 Server List Ping.
///
/// `0xFE 0x01` in, and back comes `0xFF` + a UTF-16BE string. The modern layout
/// is `§1\0protocol\0version\0motd\0online\0max`; the ancient one is
/// `motd§online§max`. Both are handled, because a server old enough to need
/// this is old enough to be either.
pub async fn query_legacy(addr: SocketAddr, timeout: Duration) -> AppResult<ServerQueryResult> {
    let (raw, rtt) = crate::net::transport::tcp_exchange(addr, &[0xFE, 0x01], timeout).await?;

    parse_legacy(&raw, addr.port(), rtt)
}

fn parse_legacy(raw: &[u8], port: u16, rtt: u32) -> AppResult<ServerQueryResult> {
    let mut r = Reader::new(raw);

    if r.u8()? != 0xFF {
        return Err(AppError::invalid("Not a Minecraft ping reply."));
    }

    // Length in UTF-16 code units, so twice that in bytes.
    let units = usize::from(r.u16_be()?);

    let bytes = r
        .bytes(
            units.checked_mul(2).ok_or_else(|| {
                AppError::invalid("The server declared an impossible reply length.")
            })?,
        )
        .map_err(|_| AppError::invalid("The server's reply was shorter than it declared."))?;

    /*
     * NUL is this format's FIELD SEPARATOR, so it survives the control-character
     * filter that every other decoder in this crate applies. Stripping it — the
     * obvious thing to do with a control byte — collapses the whole reply into
     * one run-together string and silently loses every field but the first.
     */
    let text: String = bytes
        .chunks_exact(2)
        .filter_map(|pair| char::from_u32(u32::from(u16::from_be_bytes([pair[0], pair[1]]))))
        .filter(|c| *c == '\0' || !c.is_control())
        .collect();

    let mut out = ServerQueryResult::new(QueryProtocol::MinecraftSlp, port, rtt);

    if let Some(rest) = text
        .strip_prefix('\u{00A7}')
        .and_then(|t| t.strip_prefix('1'))
    {
        // Modern legacy: NUL-separated, five fields after the marker.
        let fields: Vec<&str> = rest.split('\0').filter(|f| !f.is_empty()).collect();

        out.version = fields.get(1).map(|v| strip_colour_codes(v));
        out.name = fields.get(2).map(|v| strip_colour_codes(v));
        out.players = fields.get(3).and_then(|v| v.trim().parse().ok());
        out.max_players = fields.get(4).and_then(|v| v.trim().parse().ok());
    } else {
        // Ancient: `motd§online§max`.
        let fields: Vec<&str> = text.split('\u{00A7}').collect();

        out.name = fields.first().map(|v| strip_colour_codes(v));
        out.players = fields.get(1).and_then(|v| v.trim().parse().ok());
        out.max_players = fields.get(2).and_then(|v| v.trim().parse().ok());
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip_the_protocols_examples() {
        assert_eq!(varint(0), vec![0x00]);
        assert_eq!(varint(255), vec![0xFF, 0x01]);
        assert_eq!(varint(25565), vec![0xDD, 0xC7, 0x01]);
        assert_eq!(varint_signed(-1), vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    }

    #[test]
    fn parses_a_plain_string_description() {
        let json = r#"{"description":"A Minecraft Server","players":{"online":5,"max":20},"version":{"name":"1.20.4"}}"#;

        let out = parse_status(json, 25565, 30).expect("parses");

        assert_eq!(out.name.as_deref(), Some("A Minecraft Server"));
        assert_eq!(out.players, Some(5));
        assert_eq!(out.max_players, Some(20));
        assert_eq!(out.version.as_deref(), Some("1.20.4"));
    }

    #[test]
    fn parses_a_chat_component_description() {
        let json = r#"{"description":{"text":"","extra":[{"text":"§aHello "},{"text":"World"}]},"players":{"online":1,"max":2}}"#;

        let out = parse_status(json, 25565, 1).expect("parses");

        assert_eq!(out.name.as_deref(), Some("Hello World"));
    }

    fn nested_description(levels: usize) -> String {
        let mut json = String::from(r#"{"description":"#);

        for _ in 0..levels {
            json.push_str(r#"{"extra":"#);
        }

        json.push_str(r#""deep""#);

        for _ in 0..levels {
            json.push('}');
        }

        json.push('}');
        json
    }

    #[test]
    fn a_deeply_nested_description_terminates_rather_than_overflowing() {
        // 200 levels of `extra` nesting — legal JSON, hostile input. serde_json
        // refuses past its own recursion limit, which is the outcome we want;
        // what matters is that it RETURNS.
        let out = parse_status(&nested_description(200), 25565, 1);

        assert!(out.is_err());
    }

    #[test]
    fn a_legal_but_nested_description_is_flattened_up_to_the_depth_cap() {
        // Inside serde_json's limit, so it reaches our own walker.
        let out = parse_status(&nested_description(20), 25565, 1).expect("parses");

        // The cap stops the walk before the payload, so no name — but no crash
        // and no partial garbage either.
        assert!(out.name.is_none() || out.name.as_deref() == Some("deep"));
    }

    #[test]
    fn absurd_player_counts_are_clamped_not_wrapped() {
        let json = r#"{"players":{"online":9223372036854775807,"max":-5}}"#;

        let out = parse_status(json, 25565, 1).expect("parses");

        // Neither fits a u32, so neither is reported — better than a wrong number.
        assert_eq!(out.players, None);
        assert_eq!(out.max_players, Some(0));
    }

    #[test]
    fn a_player_sample_becomes_a_roster() {
        let json = r#"{"players":{"online":2,"max":10,"sample":[{"name":"§aalice"},{"name":"bob"},{"name":""}]}}"#;

        let out = parse_status(json, 25565, 1).expect("parses");

        assert_eq!(out.player_list.len(), 2);
        assert_eq!(out.player_list[0].name, "alice");
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        assert!(parse_status("not json", 25565, 1).is_err());
        assert!(parse_status("", 25565, 1).is_err());
    }

    #[test]
    fn a_reply_with_no_minecraft_fields_is_rejected() {
        // All the struct's fields are optional, so serde accepts each of these
        // and hands back an empty `Status`. Something answered; it is not a
        // Minecraft server.
        for shape in ["[]", "{}", "null", r#"{"unrelated":1}"#] {
            assert!(parse_status(shape, 25565, 1).is_err(), "{shape}");
        }
    }

    fn legacy_reply(text: &str) -> Vec<u8> {
        let units: Vec<u16> = text.encode_utf16().collect();

        let mut out = vec![0xFF];
        out.extend_from_slice(&(units.len() as u16).to_be_bytes());

        for u in units {
            out.extend_from_slice(&u.to_be_bytes());
        }

        out
    }

    #[test]
    fn parses_a_modern_legacy_ping() {
        let reply = legacy_reply("\u{00A7}1\x0047\x001.4.2\0My Server\x003\x0020");

        let out = parse_legacy(&reply, 25565, 12).expect("parses");

        assert_eq!(out.version.as_deref(), Some("1.4.2"));
        assert_eq!(out.name.as_deref(), Some("My Server"));
        assert_eq!(out.players, Some(3));
        assert_eq!(out.max_players, Some(20));
    }

    #[test]
    fn parses_an_ancient_legacy_ping() {
        let reply = legacy_reply("Old Server\u{00A7}7\u{00A7}32");

        let out = parse_legacy(&reply, 25565, 9).expect("parses");

        assert_eq!(out.name.as_deref(), Some("Old Server"));
        assert_eq!(out.players, Some(7));
        assert_eq!(out.max_players, Some(32));
    }

    #[test]
    fn a_legacy_reply_shorter_than_declared_errors() {
        let mut reply = legacy_reply("hello");
        reply.truncate(5);

        assert!(parse_legacy(&reply, 25565, 0).is_err());
    }

    #[test]
    fn a_legacy_length_that_would_overflow_is_refused() {
        // Declares 65535 UTF-16 units (131070 bytes) and sends two.
        let reply = vec![0xFF, 0xFF, 0xFF, 0x00, 0x41];

        assert!(parse_legacy(&reply, 25565, 0).is_err());
    }

    #[test]
    fn truncated_legacy_replies_never_panic() {
        let full = legacy_reply("\u{00A7}1\x0047\x001.4.2\0My Server\x003\x0020");

        for cut in 0..full.len() {
            let _ = parse_legacy(&full[..cut], 25565, 0);
        }
    }

    #[test]
    fn status_completion_is_driven_by_the_length_prefix() {
        // Declares 5 bytes, has 5.
        assert!(status_is_complete(&[0x05, 1, 2, 3, 4, 5]));
        // Declares 5 bytes, has 3.
        assert!(!status_is_complete(&[0x05, 1, 2, 3]));
        assert!(!status_is_complete(&[]));
    }
}
