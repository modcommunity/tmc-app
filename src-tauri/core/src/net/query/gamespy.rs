//! The three GameSpy generations, which are three unrelated protocols that
//! happen to share a name.
//!
//!   * **v1** — `\status\` in, a `\key\value\` string back. Unreal Tournament,
//!     the original Battlefield titles, Serious Sam.
//!   * **v2** — binary, with a per-section request byte selecting rules,
//!     players and teams. Battlefield 2, Unreal Tournament 2004.
//!   * **v3** — a challenge handshake followed by a NUL-separated key/value
//!     block, optionally split across packets. This is also what Minecraft's
//!     `enable-query` port speaks.
//!
//! v3's challenge is the interesting one: the server replies with a
//! **decimal string** of a signed 32-bit integer which must be echoed back as
//! four big-endian bytes. Servers send it with a trailing NUL and sometimes
//! leading whitespace, and a negative challenge is normal — parsing it as
//! unsigned is the classic bug that makes roughly half of all servers appear
//! unreachable.

use std::net::SocketAddr;
use std::time::Duration;

use crate::error::{AppError, AppResult};
use crate::net::query::{PlayerEntry, QueryProtocol, ServerQueryResult};
use crate::net::reader::{clean_text, strip_colour_codes, Reader};
use crate::net::transport::{udp_exchange, udp_exchange_multi};

const MAX_RULES: usize = 64;
const MAX_PLAYERS: usize = 128;

// --------------------------------------------------------------------- v1

pub async fn query_v1(addr: SocketAddr, timeout: Duration) -> AppResult<ServerQueryResult> {
    let (raw, rtt) = udp_exchange(addr, b"\\status\\", timeout).await?;

    parse_v1(&raw, addr.port(), rtt)
}

fn parse_v1(raw: &[u8], port: u16, rtt: u32) -> AppResult<ServerQueryResult> {
    let text = clean_text(raw);

    if !text.contains('\\') {
        return Err(AppError::invalid("Not a GameSpy reply."));
    }

    let fields: Vec<&str> = text.trim_matches('\\').split('\\').collect();

    let mut out = ServerQueryResult::new(QueryProtocol::Gamespy1, port, rtt);
    let mut players: Vec<(usize, String)> = Vec::new();

    for pair in fields.chunks(2) {
        let (Some(key), Some(value)) = (pair.first(), pair.get(1)) else {
            continue;
        };

        let lower = key.to_ascii_lowercase();

        /*
         * v1 encodes the roster as indexed keys — `player_0`, `player_1`. They
         * are collected separately and sorted by index, because the wire order
         * is not guaranteed and a roster shown out of order looks like a bug.
         */
        if let Some(index) = lower.strip_prefix("player_").and_then(|i| i.parse().ok()) {
            players.push((index, strip_colour_codes(value)));
            continue;
        }

        match lower.as_str() {
            "hostname" => out.name = Some(strip_colour_codes(value)),
            "mapname" => out.map = Some(value.to_string()),
            "gamename" | "gametype" => {
                if out.game.is_none() {
                    out.game = Some(value.to_string());
                }
            }
            "gamever" => out.version = Some(value.to_string()),
            "numplayers" => out.players = value.trim().parse().ok(),
            "maxplayers" => out.max_players = value.trim().parse().ok(),
            "password" => out.password = Some(value.trim() != "0"),
            _ => {
                if out.rules.len() < MAX_RULES {
                    out.rules.insert(key.to_string(), value.to_string());
                }
            }
        }
    }

    players.sort_by_key(|(index, _)| *index);

    out.player_list = players
        .into_iter()
        .take(MAX_PLAYERS)
        .map(|(_, name)| PlayerEntry {
            name,
            score: None,
            duration: None,
            ping: None,
        })
        .collect();

    Ok(out)
}

// --------------------------------------------------------------------- v2

/// `\xFE\xFD\x00` + a request id, then one byte each for server info, players
/// and teams. `0xFF` means "send this section", `0x00` means "skip it".
pub async fn query_v2(
    addr: SocketAddr,
    timeout: Duration,
    want_players: bool,
) -> AppResult<ServerQueryResult> {
    let players_flag = if want_players { 0xFF } else { 0x00 };

    let request = [
        0xFE,
        0xFD,
        0x00, // header
        0x04,
        0x05,
        0x06,
        0x07, // request id, echoed back
        0xFF,
        players_flag,
        0x00, // info, players, teams
    ];

    let (raw, rtt) = udp_exchange(addr, &request, timeout).await?;

    parse_v2(&raw, addr.port(), rtt)
}

fn parse_v2(raw: &[u8], port: u16, rtt: u32) -> AppResult<ServerQueryResult> {
    let mut r = Reader::new(raw);

    if r.u8()? != 0x00 {
        return Err(AppError::invalid("Not a GameSpy v2 reply."));
    }

    // The echoed request id.
    r.skip(4)?;

    let mut out = ServerQueryResult::new(QueryProtocol::Gamespy2, port, rtt);

    // Key/value pairs until an empty key.
    loop {
        if r.is_empty() {
            return Ok(out);
        }

        let key = r.cstring()?;

        if key.is_empty() {
            break;
        }

        let value = r.cstring()?;

        match key.to_ascii_lowercase().as_str() {
            "hostname" => out.name = Some(strip_colour_codes(&value)),
            "mapname" => out.map = Some(value),
            "gamename" | "gametype" => {
                if out.game.is_none() {
                    out.game = Some(value);
                }
            }
            "gamever" => out.version = Some(value),
            "numplayers" => out.players = value.trim().parse().ok(),
            "maxplayers" => out.max_players = value.trim().parse().ok(),
            "password" => out.password = Some(value.trim() != "0"),
            _ => {
                if out.rules.len() < MAX_RULES {
                    out.rules.insert(key, value);
                }
            }
        }
    }

    /*
     * The player section: a count, then a column header list terminated by an
     * empty string, then one row per player. Missing entirely when the request
     * asked for no players, so every read past here is best-effort.
     */
    if r.remaining() < 2 {
        return Ok(out);
    }

    let count = usize::from(r.u8()?).min(MAX_PLAYERS);

    let mut columns: Vec<String> = Vec::new();

    while let Ok(column) = r.cstring() {
        if column.is_empty() {
            break;
        }

        columns.push(column.trim_end_matches('_').to_ascii_lowercase());

        if columns.len() > 16 {
            break;
        }
    }

    for _ in 0..count {
        if r.is_empty() {
            break;
        }

        let mut entry = PlayerEntry {
            name: String::new(),
            score: None,
            duration: None,
            ping: None,
        };

        for column in &columns {
            let Ok(value) = r.cstring() else { break };

            match column.as_str() {
                "player" => entry.name = strip_colour_codes(&value),
                "score" => entry.score = value.trim().parse().ok(),
                "ping" => entry.ping = value.trim().parse().ok(),
                _ => {}
            }
        }

        if !entry.name.is_empty() {
            out.player_list.push(entry);
        }
    }

    Ok(out)
}

// --------------------------------------------------------------------- v3

const V3_MAGIC: [u8; 2] = [0xFE, 0xFD];
const V3_SESSION: [u8; 4] = [0x01, 0x02, 0x03, 0x04];

pub async fn query_v3(
    addr: SocketAddr,
    timeout: Duration,
    want_players: bool,
) -> AppResult<ServerQueryResult> {
    // Handshake: type 9, session id, no payload.
    let mut handshake = Vec::with_capacity(7);
    handshake.extend_from_slice(&V3_MAGIC);
    handshake.push(0x09);
    handshake.extend_from_slice(&V3_SESSION);

    let (challenge_raw, rtt1) = udp_exchange(addr, &handshake, timeout).await?;

    let token = parse_v3_challenge(&challenge_raw)?;

    // Stat request: type 0, session id, the challenge token, then four padding
    // bytes that ask for the full (multi-packet) reply rather than the short one.
    let mut request = Vec::with_capacity(15);
    request.extend_from_slice(&V3_MAGIC);
    request.push(0x00);
    request.extend_from_slice(&V3_SESSION);
    request.extend_from_slice(&token.to_be_bytes());

    if want_players {
        request.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    }

    let (packets, rtt2) = udp_exchange_multi(addr, &request, timeout, 8).await?;

    // The RTT that matters is the whole exchange, both round trips.
    let mut result = parse_v3(&packets, addr.port(), rtt1.saturating_add(rtt2))?;
    result.protocol = QueryProtocol::Gamespy3;

    Ok(result)
}

/// Read the challenge token.
///
/// **Signed.** The server sends a decimal string of an `i32`, and about half of
/// them are negative. Parsing as `u32` fails outright on those, which presents
/// as "this server never answers" — the single most common GameSpy v3 bug.
fn parse_v3_challenge(raw: &[u8]) -> AppResult<i32> {
    let mut r = Reader::new(raw);

    if r.u8()? != 0x09 {
        return Err(AppError::invalid("Not a GameSpy v3 challenge."));
    }

    r.skip(4)?; // echoed session id

    let text = r.cstring()?;

    text.trim()
        .parse::<i32>()
        .map_err(|_| AppError::invalid("The server sent an unreadable challenge."))
}

fn parse_v3(packets: &[Vec<u8>], port: u16, rtt: u32) -> AppResult<ServerQueryResult> {
    let mut out = ServerQueryResult::new(QueryProtocol::Gamespy3, port, rtt);

    /*
     * Concatenate the payloads in arrival order.
     *
     * v3 fragments carry a split header the spec describes inconsistently and
     * that several implementations get wrong, so the header is skipped by a
     * fixed offset rather than trusted, and the pieces are joined in the order
     * they arrived. In practice servers send them in order on a LAN-quality
     * path; a reordered reply degrades to a partial parse rather than to
     * garbage, because every field is a self-describing key/value pair.
     */
    let mut body: Vec<u8> = Vec::new();

    for (index, packet) in packets.iter().enumerate() {
        let mut r = Reader::new(packet);

        if r.u8()? != 0x00 {
            continue;
        }

        r.skip(4)?; // session id

        // The "splitnum" marker and its index byte, present on multi-packet
        // replies only.
        let rest = r.rest();

        let payload = match find_subslice(rest, b"splitnum\0") {
            Some(at) => rest.get(at + 9 + 2..).unwrap_or(&[]),
            // The first packet of a single-packet reply has neither.
            None if index == 0 => rest,
            None => rest,
        };

        body.extend_from_slice(payload);
    }

    if body.is_empty() {
        return Err(AppError::invalid("The server sent an empty reply."));
    }

    /*
     * The body is `key\0value\0…\0\0` then optionally `\x01player_\0\0` and the
     * roster. Split on the roster marker first so a player named `hostname`
     * cannot be read as a rule.
     */
    let (rules_part, players_part) = match find_subslice(&body, b"\x01player_\0") {
        Some(at) => (
            body.get(..at).unwrap_or(&[]).to_vec(),
            body.get(at + 9..).unwrap_or(&[]).to_vec(),
        ),
        None => (body.clone(), Vec::new()),
    };

    let mut r = Reader::new(&rules_part);

    while !r.is_empty() {
        let Ok(key) = r.cstring() else { break };

        if key.is_empty() {
            break;
        }

        let Ok(value) = r.cstring() else { break };

        match key.to_ascii_lowercase().as_str() {
            "hostname" => out.name = Some(strip_colour_codes(&value)),
            "map" | "mapname" => out.map = Some(value),
            "gametype" | "game_id" | "gamename" => {
                if out.game.is_none() {
                    out.game = Some(value);
                }
            }
            "version" | "gamever" => out.version = Some(value),
            "numplayers" => out.players = value.trim().parse().ok(),
            "maxplayers" => out.max_players = value.trim().parse().ok(),
            "password" => out.password = Some(value.trim() != "0"),
            _ => {
                if out.rules.len() < MAX_RULES {
                    out.rules.insert(key, value);
                }
            }
        }
    }

    let mut pr = Reader::new(&players_part);

    // A leading empty string separates the column header from the values.
    let _ = pr.cstring();

    while !pr.is_empty() && out.player_list.len() < MAX_PLAYERS {
        let Ok(name) = pr.cstring() else { break };

        if name.is_empty() {
            break;
        }

        out.player_list.push(PlayerEntry {
            name: strip_colour_codes(&name),
            score: None,
            duration: None,
            ping: None,
        });
    }

    Ok(out)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }

    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------ v1

    #[test]
    fn parses_a_v1_status_reply() {
        let raw = b"\\hostname\\UT Server\\mapname\\DM-Deck16\\numplayers\\6\\maxplayers\\16\\gamever\\436\\password\\0\\player_1\\bob\\player_0\\alice\\";

        let out = parse_v1(raw, 7777, 20).expect("parses");

        assert_eq!(out.name.as_deref(), Some("UT Server"));
        assert_eq!(out.map.as_deref(), Some("DM-Deck16"));
        assert_eq!(out.players, Some(6));
        assert_eq!(out.max_players, Some(16));
        assert_eq!(out.password, Some(false));

        // Sorted by index, not wire order.
        assert_eq!(out.player_list[0].name, "alice");
        assert_eq!(out.player_list[1].name, "bob");
    }

    #[test]
    fn a_v1_reply_with_no_delimiter_is_rejected() {
        assert!(parse_v1(b"nonsense", 7777, 0).is_err());
    }

    // ------------------------------------------------------------------ v2

    fn v2_reply() -> Vec<u8> {
        let mut b = vec![0x00, 0x04, 0x05, 0x06, 0x07];
        b.extend_from_slice(b"hostname\0BF2 Server\0");
        b.extend_from_slice(b"mapname\0strike_at_karkand\0");
        b.extend_from_slice(b"numplayers\x0032\0");
        b.extend_from_slice(b"maxplayers\x0064\0");
        b.push(0); // end of rules
        b.push(2); // player count
        b.extend_from_slice(b"player_\0score_\0\0");
        b.extend_from_slice(b"alice\x0010\0");
        b.extend_from_slice(b"bob\0-3\0");
        b
    }

    #[test]
    fn parses_a_v2_reply_with_a_roster() {
        let out = parse_v2(&v2_reply(), 29900, 33).expect("parses");

        assert_eq!(out.name.as_deref(), Some("BF2 Server"));
        assert_eq!(out.map.as_deref(), Some("strike_at_karkand"));
        assert_eq!(out.players, Some(32));
        assert_eq!(out.player_list.len(), 2);
        assert_eq!(out.player_list[0].name, "alice");
        assert_eq!(out.player_list[0].score, Some(10));
        assert_eq!(out.player_list[1].score, Some(-3));
    }

    #[test]
    fn a_v2_reply_with_no_player_section_still_parses() {
        let mut b = vec![0x00, 0x04, 0x05, 0x06, 0x07];
        b.extend_from_slice(b"hostname\0Bare\0");
        b.push(0);

        let out = parse_v2(&b, 29900, 1).expect("parses");

        assert_eq!(out.name.as_deref(), Some("Bare"));
        assert!(out.player_list.is_empty());
    }

    #[test]
    fn truncated_v2_replies_never_panic() {
        let full = v2_reply();

        for cut in 0..full.len() {
            let _ = parse_v2(&full[..cut], 29900, 0);
        }
    }

    // ------------------------------------------------------------------ v3

    #[test]
    fn a_negative_challenge_parses() {
        // This is the bug that makes half of all servers look unreachable.
        let mut b = vec![0x09, 0x01, 0x02, 0x03, 0x04];
        b.extend_from_slice(b"-1795675819\0");

        assert_eq!(parse_v3_challenge(&b).expect("parses"), -1795675819);
    }

    #[test]
    fn a_positive_challenge_parses() {
        let mut b = vec![0x09, 0x01, 0x02, 0x03, 0x04];
        b.extend_from_slice(b"  123456\0");

        assert_eq!(parse_v3_challenge(&b).expect("parses"), 123456);
    }

    #[test]
    fn a_non_numeric_challenge_is_an_error() {
        let mut b = vec![0x09, 0x01, 0x02, 0x03, 0x04];
        b.extend_from_slice(b"nope\0");

        assert!(parse_v3_challenge(&b).is_err());
        assert!(parse_v3_challenge(&[0x00]).is_err());
        assert!(parse_v3_challenge(&[]).is_err());
    }

    fn v3_packet() -> Vec<u8> {
        let mut b = vec![0x00, 0x01, 0x02, 0x03, 0x04];
        b.extend_from_slice(b"splitnum\0\x00\x00");
        b.extend_from_slice(b"hostname\0A Minecraft Server\0");
        b.extend_from_slice(b"gametype\0SMP\0");
        b.extend_from_slice(b"map\0world\0");
        b.extend_from_slice(b"numplayers\x003\0");
        b.extend_from_slice(b"maxplayers\x0020\0");
        b.extend_from_slice(b"version\x001.20.4\0");
        b.extend_from_slice(b"\0");
        b.extend_from_slice(b"\x01player_\0\0");
        b.extend_from_slice(b"alice\0bob\0carol\0\0");
        b
    }

    #[test]
    fn parses_a_v3_reply_with_a_roster() {
        let out = parse_v3(&[v3_packet()], 25565, 40).expect("parses");

        assert_eq!(out.name.as_deref(), Some("A Minecraft Server"));
        assert_eq!(out.map.as_deref(), Some("world"));
        assert_eq!(out.players, Some(3));
        assert_eq!(out.max_players, Some(20));
        assert_eq!(out.version.as_deref(), Some("1.20.4"));

        let names: Vec<&str> = out.player_list.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["alice", "bob", "carol"]);
    }

    #[test]
    fn a_player_named_hostname_is_not_read_as_a_rule() {
        let mut b = vec![0x00, 0x01, 0x02, 0x03, 0x04];
        b.extend_from_slice(b"splitnum\0\x00\x00");
        b.extend_from_slice(b"hostname\0Real Name\0\0");
        b.extend_from_slice(b"\x01player_\0\0");
        b.extend_from_slice(b"hostname\0\0");

        let out = parse_v3(&[b], 25565, 1).expect("parses");

        assert_eq!(out.name.as_deref(), Some("Real Name"));
        assert_eq!(out.player_list[0].name, "hostname");
    }

    #[test]
    fn an_empty_v3_reply_is_an_error() {
        assert!(parse_v3(&[], 25565, 0).is_err());
        assert!(parse_v3(&[vec![0x00, 1, 2, 3, 4]], 25565, 0).is_err());
    }

    #[test]
    fn truncated_v3_packets_never_panic() {
        let full = v3_packet();

        for cut in 0..full.len() {
            let _ = parse_v3(&[full[..cut].to_vec()], 25565, 0);
        }
    }

    #[test]
    fn subslice_search_handles_the_edges() {
        assert_eq!(find_subslice(b"abcdef", b"cd"), Some(2));
        assert_eq!(find_subslice(b"abc", b"abcd"), None);
        assert_eq!(find_subslice(b"", b"a"), None);
        assert_eq!(find_subslice(b"abc", b""), None);
    }
}
