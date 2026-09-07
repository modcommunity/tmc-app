//! DICE's Frostbite R-CON — Battlefield: Bad Company 2, Battlefield 3,
//! Battlefield 4, Battlefield Hardline.
//!
//! A length-framed binary protocol over TCP. Every packet is
//!
//! ```text
//! u32 LE  sequence      -- id, plus the response/origin flag bits
//! u32 LE  size          -- TOTAL packet size, header included
//! u32 LE  numWords
//! Word[]  words         -- each: u32 LE length, that many bytes, one NUL
//! ```
//!
//! and a word's length prefix **excludes** the NUL that follows it.
//!
//! The two request packets below are byte-for-byte the ones GameQ has sent for
//! a decade, sequence flags included. They are copied rather than composed
//! because the flag bits in the sequence field are the part every independent
//! reimplementation gets subtly wrong, and a server that dislikes them simply
//! never answers — indistinguishable from a dead box.
//!
//! **`size` counts the header.** This is the one place the framing invites an
//! off-by-four: after reading the 8-byte sequence-and-size prefix, the body
//! still to come is `size - 8`, not `size - 4`. Reading `size - 4` blocks
//! waiting for four bytes that only arrive with the NEXT packet — and since
//! nothing follows an unsolicited `serverInfo` reply, the query hangs until the
//! deadline and every Battlefield server reports as timed out. `spy`'s Go
//! implementation has exactly that bug (`internal/protocols/frostbite.go`), so
//! it is not a hypothetical.
//!
//! ## Ports
//!
//! R-CON does not listen on the game port. `spy/internal/protocols/frostbite.go`
//! uses **game port + 22000** when the row carries no explicit query port, and
//! `QueryTarget::resolve_port` carries that same exception.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{AppError, AppResult};
use crate::net::query::{PlayerEntry, QueryProtocol, ServerQueryResult};
use crate::net::reader::Reader;
use crate::net::transport::elapsed_ms;

/// `serverInfo`, one word. 27 bytes, which is what its own `size` field says.
const REQ_SERVER_INFO: &[u8] =
    b"\x00\x00\x00\x21\x1b\x00\x00\x00\x01\x00\x00\x00\x0a\x00\x00\x00serverInfo\x00";

/// `listPlayers all`, two words. 36 bytes.
const REQ_LIST_PLAYERS: &[u8] = b"\x00\x00\x00\x23\x24\x00\x00\x00\x02\x00\x00\x00\x0b\x00\x00\x00listPlayers\x00\x03\x00\x00\x00all\x00";

/// One response body. The largest real packet is a full 64-slot roster with
/// nine tags per player; 1MB is far above that and far below what a hostile
/// `size` field can ask for.
const MAX_PACKET: usize = 1 << 20;

/// The header that `size` counts but [`read_packet`] has already consumed.
const HEADER_BYTES: u32 = 8;

/// Words in one response. A roster is `2 + tags + 1 + players * tags`; at 64
/// players and nine tags that is 588.
const MAX_WORDS: usize = 4_096;

/// Roster length. Frostbite servers top out at 64 slots; the cap is only here
/// so a lying `playerCount` cannot drive an allocation.
const MAX_PLAYERS: usize = 256;

/// Tags per player, from the roster header.
const MAX_TAGS: usize = 32;

pub async fn query(
    addr: SocketAddr,
    timeout: Duration,
    want_players: bool,
) -> AppResult<ServerQueryResult> {
    let started = Instant::now();

    let mut stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| AppError::Network("The server did not answer.".into()))??;

    // Nagle would hold the 27-byte request waiting for data that is never
    // coming, adding ~40ms to every measurement.
    let _ = stream.set_nodelay(true);

    let rtt = elapsed_ms(started);

    let info = exchange(&mut stream, REQ_SERVER_INFO, timeout).await?;
    let words = decode_words(&info)?;

    let mut out = parse_server_info(&words, addr.port(), rtt)?;

    /*
     * The roster is a second round trip on the same connection, so it is only
     * made when the caller asked. A grid of fifty cards does not want it, and
     * a server that answers `serverInfo` but refuses `listPlayers` must still
     * render as online — hence the error is swallowed rather than propagated.
     */
    if want_players {
        if let Ok(raw) = exchange(&mut stream, REQ_LIST_PLAYERS, timeout).await {
            if let Ok(words) = decode_words(&raw) {
                out.player_list = parse_player_list(&words);
            }
        }
    }

    Ok(out)
}

/// Write one request and read exactly one framed reply back.
async fn exchange(stream: &mut TcpStream, request: &[u8], timeout: Duration) -> AppResult<Vec<u8>> {
    tokio::time::timeout(timeout, stream.write_all(request))
        .await
        .map_err(|_| AppError::Network("The server stopped responding.".into()))??;

    read_packet(stream, timeout).await
}

/// Read one packet, returning the body from `numWords` onwards.
async fn read_packet(stream: &mut TcpStream, timeout: Duration) -> AppResult<Vec<u8>> {
    let mut header = [0u8; 8];

    tokio::time::timeout(timeout, stream.read_exact(&mut header))
        .await
        .map_err(|_| AppError::Network("The server did not answer.".into()))?
        .map_err(|_| AppError::Network("The server did not answer.".into()))?;

    // Bytes 0..4 are the sequence. The response flag is not checked: this
    // connection carries one request at a time, so there is nothing to match a
    // reply against, and several server builds echo flags of their own.
    let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);

    let remaining = size
        .checked_sub(HEADER_BYTES)
        .ok_or_else(|| AppError::invalid("The server sent a truncated packet."))?
        as usize;

    if remaining > MAX_PACKET {
        return Err(AppError::invalid("The server sent an oversized packet."));
    }

    let mut body = vec![0u8; remaining];

    if remaining > 0 {
        tokio::time::timeout(timeout, stream.read_exact(&mut body))
            .await
            .map_err(|_| AppError::Network("The server stopped responding.".into()))?
            .map_err(|_| AppError::Network("The server stopped responding.".into()))?;
    }

    Ok(body)
}

/// `numWords`, then that many length-prefixed, NUL-terminated words.
///
/// A word count is the server's claim, so it bounds the loop only alongside
/// [`MAX_WORDS`] and the buffer itself; a truncated body ends the loop instead
/// of erroring, because a partial word list still carries the hostname and the
/// player counts that precede it.
fn decode_words(body: &[u8]) -> AppResult<Vec<String>> {
    let mut reader = Reader::new(body);

    let count = reader.u32_le()? as usize;

    let mut words = Vec::with_capacity(count.min(64));

    for _ in 0..count.min(MAX_WORDS) {
        let Ok(word) = reader.pstring(4) else { break };

        words.push(word);

        // The NUL the length prefix did not count. Absent on a truncated tail.
        if reader.u8().is_err() {
            break;
        }
    }

    Ok(words)
}

/// ```text
/// [0] "OK"          [4] gamemode
/// [1] hostname      [5] map
/// [2] players       [6] roundsPlayed
/// [3] maxPlayers    [7] roundsTotal
/// ```
fn parse_server_info(words: &[String], port: u16, rtt: u32) -> AppResult<ServerQueryResult> {
    match words.first().map(String::as_str) {
        Some("OK") => {}
        // `UnknownCommand`, `InvalidPasswordHash`, and friends. The socket is
        // open and something is listening, but it is not answering the
        // question, so reporting it as a live server would be a lie.
        Some(other) => {
            return Err(AppError::invalid(format!(
                "The server refused the query ({other})."
            )))
        }
        None => return Err(AppError::invalid("Not a Frostbite reply.")),
    }

    let mut out = ServerQueryResult::new(QueryProtocol::Frostbite, port, rtt);

    out.name = words.get(1).filter(|s| !s.is_empty()).cloned();
    out.players = words.get(2).and_then(|s| s.parse().ok());
    out.max_players = words.get(3).and_then(|s| s.parse().ok());
    out.game = words.get(4).filter(|s| !s.is_empty()).cloned();

    // Lowercased to match `spy`, which stores `ServerMap.Name` that way — the
    // app's map art is looked up by that key.
    out.map = words
        .get(5)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase());

    if let Some(gamemode) = words.get(4).filter(|s| !s.is_empty()) {
        out.rules.insert("gamemode".into(), gamemode.clone());
    }

    if let (Some(played), Some(total)) = (words.get(6), words.get(7)) {
        if !played.is_empty() && !total.is_empty() {
            out.rules
                .insert("rounds".into(), format!("{played}/{total}"));
        }
    }

    /*
     * `serverInfo` carries no password field at any index — Frostbite reports
     * that through `vars.gamePassword`, an authenticated command. It stays
     * `None`, because "this protocol does not say" and "no password" are
     * different facts and rendering the second for the first is how a browser
     * sends someone to a server they cannot join.
     */

    Ok(out)
}

/// ```text
/// [0] "OK"   [1] tagCount   [2..] tag names   [n] playerCount   [n+1..] rows
/// ```
///
/// Column meaning comes from the tag names, never from position: the tag set
/// differs between Bad Company 2, BF3 and BF4, and assuming BF4's order is how
/// a squad id ends up rendered as a score.
fn parse_player_list(words: &[String]) -> Vec<PlayerEntry> {
    if words.first().map(String::as_str) != Some("OK") {
        return Vec::new();
    }

    let Some(tag_count) = words.get(1).and_then(|s| s.parse::<usize>().ok()) else {
        return Vec::new();
    };

    if tag_count == 0 || tag_count > MAX_TAGS {
        return Vec::new();
    }

    let Some(tags) = words.get(2..2 + tag_count) else {
        return Vec::new();
    };

    let Some(count) = words
        .get(2 + tag_count)
        .and_then(|s| s.parse::<usize>().ok())
    else {
        return Vec::new();
    };

    let start = 2 + tag_count + 1;

    let mut players = Vec::with_capacity(count.min(64));

    for i in 0..count.min(MAX_PLAYERS) {
        let offset = start + i * tag_count;

        // A short tail ends the roster rather than emitting half a row.
        let Some(row) = words.get(offset..offset + tag_count) else {
            break;
        };

        let mut entry = PlayerEntry {
            name: String::new(),
            score: None,
            duration: None,
            ping: None,
        };

        for (tag, value) in tags.iter().zip(row) {
            match tag.as_str() {
                "name" => entry.name = value.clone(),
                "score" => entry.score = value.parse().ok(),
                // 65535 is Frostbite's "not measured yet", which a fresh joiner
                // reports for a few seconds.
                "ping" => entry.ping = value.parse().ok().filter(|p| *p < 65_535),
                _ => {}
            }
        }

        // Frostbite lists a slot that is connecting but has not picked a name.
        if entry.name.is_empty() {
            continue;
        }

        players.push(entry);
    }

    players
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a word list into a full packet, header included.
    fn packet(words: &[&str]) -> Vec<u8> {
        let mut body = Vec::new();

        body.extend_from_slice(&(words.len() as u32).to_le_bytes());

        for word in words {
            body.extend_from_slice(&(word.len() as u32).to_le_bytes());
            body.extend_from_slice(word.as_bytes());
            body.push(0);
        }

        let mut out = Vec::new();

        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32 + HEADER_BYTES).to_le_bytes());
        out.extend_from_slice(&body);

        out
    }

    /// What `read_packet` hands to `decode_words`.
    fn body(words: &[&str]) -> Vec<u8> {
        packet(words)[8..].to_vec()
    }

    #[test]
    fn the_request_packets_declare_their_own_true_length() {
        // Both are GameQ's bytes. If a future edit rewrites a command name
        // without fixing the `size` field, the server waits forever for the
        // rest of a packet that has already been sent in full.
        for request in [REQ_SERVER_INFO, REQ_LIST_PLAYERS] {
            let declared = u32::from_le_bytes([request[4], request[5], request[6], request[7]]);

            assert_eq!(declared as usize, request.len());
        }
    }

    #[test]
    fn a_request_round_trips_through_the_reply_decoder() {
        // The request framing and the response framing are the same framing,
        // so the requests double as fixtures.
        let words = decode_words(&REQ_LIST_PLAYERS[8..]).unwrap();

        assert_eq!(words, vec!["listPlayers", "all"]);
    }

    #[test]
    fn server_info_maps_every_documented_slot() {
        let words = decode_words(&body(&[
            "OK",
            "Nightly Test Server",
            "24",
            "64",
            "ConquestLarge0",
            "MP_Siege",
            "1",
            "2",
        ]))
        .unwrap();

        let out = parse_server_info(&words, 47200, 12).unwrap();

        assert!(out.online);
        assert_eq!(out.rtt_ms, 12);
        assert_eq!(out.protocol, QueryProtocol::Frostbite);
        assert_eq!(out.name.as_deref(), Some("Nightly Test Server"));
        assert_eq!(out.players, Some(24));
        assert_eq!(out.max_players, Some(64));
        assert_eq!(out.game.as_deref(), Some("ConquestLarge0"));
        assert_eq!(out.map.as_deref(), Some("mp_siege"));
        assert_eq!(out.rules.get("rounds").map(String::as_str), Some("1/2"));
        // Not reported by this command, so not claimed either way.
        assert_eq!(out.password, None);
    }

    #[test]
    fn a_refusal_is_an_error_not_an_online_server() {
        let words = decode_words(&body(&["UnknownCommand"])).unwrap();

        assert!(parse_server_info(&words, 47200, 1).is_err());
    }

    #[test]
    fn an_empty_reply_is_an_error() {
        assert!(parse_server_info(&[], 47200, 1).is_err());
    }

    #[test]
    fn player_columns_follow_the_tag_names_not_their_position() {
        // Deliberately not BF4's order: score before name, ping in the middle.
        let words = decode_words(&body(&[
            "OK", "4", "score", "ping", "name", "squadId", //
            "2",       //
            "300", "45", "alice", "1", //
            "10", "65535", "bob", "2",
        ]))
        .unwrap();

        let players = parse_player_list(&words);

        assert_eq!(players.len(), 2);
        assert_eq!(players[0].name, "alice");
        assert_eq!(players[0].score, Some(300));
        assert_eq!(players[0].ping, Some(45));
        assert_eq!(players[1].name, "bob");
        // 65535 is "not measured yet", not a 65-second ping.
        assert_eq!(players[1].ping, None);
    }

    #[test]
    fn a_roster_shorter_than_its_own_count_stops_at_the_data() {
        let words = decode_words(&body(&[
            "OK", "2", "name", "ping", //
            "9",    // claims nine players…
            "alice", "10", "bob", "20", // …and supplies two.
        ]))
        .unwrap();

        assert_eq!(parse_player_list(&words).len(), 2);
    }

    #[test]
    fn a_lying_tag_count_yields_no_players_rather_than_a_panic() {
        for tag_count in ["0", "999", "-1", "not-a-number"] {
            let words = decode_words(&body(&["OK", tag_count, "name", "1", "alice"])).unwrap();

            assert!(parse_player_list(&words).is_empty());
        }
    }

    /// Mandatory for every protocol in this module: feed the parser every
    /// prefix of a valid reply and require that none of them panic.
    #[test]
    fn every_truncation_of_a_valid_reply_is_handled() {
        let info = body(&["OK", "Server", "8", "16", "Rush0", "MP_Subway", "1", "2"]);

        let roster = body(&[
            "OK", "3", "name", "score", "ping", "2", "alice", "10", "20", "bob", "30", "40",
        ]);

        for raw in [info, roster] {
            for cut in 0..=raw.len() {
                let prefix = &raw[..cut];

                let words = decode_words(prefix).unwrap_or_default();

                let _ = parse_server_info(&words, 47200, 1);
                let _ = parse_player_list(&words);
            }
        }
    }

    #[test]
    fn an_absurd_word_count_does_not_allocate_against_it() {
        // Word count of 4 billion, no words. The loop is bounded by MAX_WORDS
        // and by the buffer, and `with_capacity` is bounded independently.
        let mut raw = u32::MAX.to_le_bytes().to_vec();
        raw.extend_from_slice(&4u32.to_le_bytes());
        raw.extend_from_slice(b"OK\0");

        let words = decode_words(&raw).unwrap();

        assert!(words.len() <= MAX_WORDS);
    }
}
