//! id Tech's `getstatus` — Quake 3 and everything descended from it: the
//! Call of Duty series, Return to Castle Wolfenstein, Enemy Territory,
//! Xonotic, OpenArena, Soldier of Fortune II.
//!
//! The wire format is four `0xFF` bytes, the literal `statusResponse`, then a
//! backslash-delimited key/value string and one line per player. It is the
//! simplest protocol here and also the one most likely to be malformed, since
//! every engine fork made its own small changes — so the parser is written to
//! extract what it recognises and ignore the rest rather than to validate.

use std::net::SocketAddr;
use std::time::Duration;

use crate::error::{AppError, AppResult};
use crate::net::query::{PlayerEntry, QueryProtocol, ServerQueryResult};
use crate::net::reader::strip_colour_codes;
use crate::net::transport::udp_exchange;

const REQUEST: &[u8] = b"\xFF\xFF\xFF\xFFgetstatus\n";

/// Rule pairs kept. Quake servers publish a lot of cvars; the UI shows a
/// handful and the rest are noise.
const MAX_RULES: usize = 64;

const MAX_PLAYERS: usize = 128;

pub async fn query(addr: SocketAddr, timeout: Duration) -> AppResult<ServerQueryResult> {
    let (raw, rtt) = udp_exchange(addr, REQUEST, timeout).await?;

    parse(&raw, addr.port(), rtt)
}

fn parse(raw: &[u8], port: u16, rtt: u32) -> AppResult<ServerQueryResult> {
    let text = clean_text_preserving_newlines(raw);

    let body = text
        .strip_prefix("\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}")
        .unwrap_or(&text);

    let body = body.trim_start_matches(['\u{FFFD}', '\u{FF}']).trim_start();

    let rest = body
        .strip_prefix("statusResponse")
        .or_else(|| body.strip_prefix("infoResponse"))
        .ok_or_else(|| AppError::invalid("Not a Quake-style status reply."))?;

    let mut lines = rest.trim_start_matches('\n').split('\n');

    let rules_line = lines
        .next()
        .ok_or_else(|| AppError::invalid("The server sent an empty status reply."))?;

    let mut out = ServerQueryResult::new(QueryProtocol::Quake3, port, rtt);

    /*
     * `\key\value\key\value`. Split on the delimiter and pair up; an odd
     * trailing key is dropped rather than paired with the next line's value.
     */
    let fields: Vec<&str> = rules_line.trim_matches('\\').split('\\').collect();

    for pair in fields.chunks(2) {
        let (Some(key), Some(value)) = (pair.first(), pair.get(1)) else {
            continue;
        };

        if key.is_empty() {
            continue;
        }

        match key.to_ascii_lowercase().as_str() {
            "sv_hostname" | "hostname" => out.name = Some(strip_colour_codes(value)),
            "mapname" | "map" => out.map = Some(value.to_string()),
            "gamename" | "game" | "gametype" => {
                if out.game.is_none() {
                    out.game = Some(value.to_string());
                }
            }
            "shortversion" | "version" | "protocol" => {
                if out.version.is_none() {
                    out.version = Some(value.to_string());
                }
            }
            "clients" | "sv_currentclients" => out.players = value.trim().parse().ok(),
            "sv_maxclients" | "sv_maxplayers" | "maxclients" => {
                out.max_players = value.trim().parse().ok()
            }
            "bots" | "sv_botcount" => out.bots = value.trim().parse().ok(),
            "pswrd" | "sv_privateclients" | "needpass" | "g_needpass" => {
                out.password = Some(value.trim() != "0" && !value.trim().is_empty())
            }
            "sv_punkbuster" | "pure" | "sv_pure" => out.secure = Some(value.trim() != "0"),
            _ => {
                if out.rules.len() < MAX_RULES {
                    out.rules.insert(key.to_string(), value.to_string());
                }
            }
        }
    }

    /*
     * Player lines: `score ping "name"`. Parsed leniently — a name containing
     * a quote (which nothing forbids) would break a strict split, so the name
     * is everything between the first and last quote on the line.
     */
    for line in lines.take(MAX_PLAYERS) {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        let mut parts = line.splitn(3, ' ');

        let score = parts.next().and_then(|v| v.parse::<i32>().ok());
        let ping = parts.next().and_then(|v| v.parse::<u32>().ok());

        let Some(raw_name) = parts.next() else {
            continue;
        };

        let name = raw_name
            .trim()
            .trim_start_matches('"')
            .trim_end_matches('"')
            .to_string();

        if name.is_empty() {
            continue;
        }

        out.player_list.push(PlayerEntry {
            name: strip_colour_codes(&name),
            score,
            duration: None,
            ping,
        });
    }

    // Several forks omit the client count and only send the roster.
    if out.players.is_none() && !out.player_list.is_empty() {
        out.players = Some(out.player_list.len() as u32);
    }

    Ok(out)
}

/// Like [`clean_text`] but keeps `\n`, which is this protocol's record
/// separator and would otherwise be stripped as a control character.
fn clean_text_preserving_newlines(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .take(32 * 1024)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply() -> Vec<u8> {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF];
        b.extend_from_slice(
            b"statusResponse\n\\sv_hostname\\^1Frag ^7Palace\\mapname\\q3dm17\\sv_maxclients\\16\\clients\\4\\g_needpass\\0\\sv_pure\\1\\g_gametype\\0\n",
        );
        b.extend_from_slice(b"12 45 \"alice\"\n");
        b.extend_from_slice(b"3 120 \"^2bob\"\n");
        b
    }

    #[test]
    fn parses_a_status_response() {
        let out = parse(&reply(), 27960, 55).expect("parses");

        assert!(out.online);
        assert_eq!(out.rtt_ms, 55);
        assert_eq!(out.name.as_deref(), Some("Frag Palace"));
        assert_eq!(out.map.as_deref(), Some("q3dm17"));
        assert_eq!(out.players, Some(4));
        assert_eq!(out.max_players, Some(16));
        assert_eq!(out.password, Some(false));
        assert_eq!(out.secure, Some(true));
    }

    #[test]
    fn parses_the_player_lines() {
        let out = parse(&reply(), 27960, 1).expect("parses");

        assert_eq!(out.player_list.len(), 2);
        assert_eq!(out.player_list[0].name, "alice");
        assert_eq!(out.player_list[0].score, Some(12));
        assert_eq!(out.player_list[0].ping, Some(45));
        assert_eq!(out.player_list[1].name, "bob");
    }

    #[test]
    fn a_missing_client_count_falls_back_to_the_roster_length() {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF];
        b.extend_from_slice(b"statusResponse\n\\sv_hostname\\X\\sv_maxclients\\8\n");
        b.extend_from_slice(b"1 10 \"solo\"\n");

        let out = parse(&b, 27960, 1).expect("parses");

        assert_eq!(out.players, Some(1));
    }

    #[test]
    fn a_name_containing_spaces_survives() {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF];
        b.extend_from_slice(b"statusResponse\n\\sv_hostname\\X\n");
        b.extend_from_slice(b"1 10 \"a player with spaces\"\n");

        let out = parse(&b, 27960, 1).expect("parses");

        assert_eq!(out.player_list[0].name, "a player with spaces");
    }

    #[test]
    fn an_odd_trailing_key_is_dropped_not_paired() {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF];
        b.extend_from_slice(b"statusResponse\n\\sv_hostname\\X\\dangling\n");

        let out = parse(&b, 27960, 1).expect("parses");

        assert_eq!(out.name.as_deref(), Some("X"));
        assert!(!out.rules.contains_key("dangling"));
    }

    #[test]
    fn a_non_quake_reply_is_rejected() {
        assert!(parse(b"garbage", 27960, 0).is_err());
        assert!(parse(&[], 27960, 0).is_err());
    }

    #[test]
    fn truncated_replies_never_panic() {
        let full = reply();

        for cut in 0..full.len() {
            let _ = parse(&full[..cut], 27960, 0);
        }
    }

    #[test]
    fn the_rules_map_is_bounded() {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF];
        b.extend_from_slice(b"statusResponse\n");

        for i in 0..500 {
            b.extend_from_slice(format!("\\key{i}\\value{i}").as_bytes());
        }
        b.push(b'\n');

        let out = parse(&b, 27960, 1).expect("parses");

        assert!(out.rules.len() <= MAX_RULES);
    }

    #[test]
    fn the_roster_is_bounded() {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF];
        b.extend_from_slice(b"statusResponse\n\\sv_hostname\\X\n");

        for i in 0..500 {
            b.extend_from_slice(format!("1 10 \"p{i}\"\n").as_bytes());
        }

        let out = parse(&b, 27960, 1).expect("parses");

        assert!(out.player_list.len() <= MAX_PLAYERS);
    }
}
