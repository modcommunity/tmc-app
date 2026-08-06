//! San Andreas Multiplayer, and the open.mp servers that kept its protocol.
//!
//! The request is the literal `SAMP`, the server's own IPv4 address as four
//! bytes, its port as two little-endian bytes, and an opcode. The server
//! **echoes that header back** and appends the payload.
//!
//! The address-in-the-request detail is why this protocol is IPv4-only here: a
//! SA-MP server has nowhere to put a v6 address in the header, so a v6-resolved
//! host is refused rather than sent a malformed query it would silently drop.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::error::{AppError, AppResult};
use crate::net::query::{QueryProtocol, ServerQueryResult};
use crate::net::reader::{clean_text, Reader};
use crate::net::transport::udp_exchange;

const OPCODE_INFO: u8 = b'i';

/// Header length: `SAMP` + 4 address bytes + 2 port bytes + 1 opcode.
const HEADER_LEN: usize = 11;

pub async fn query(addr: SocketAddr, timeout: Duration) -> AppResult<ServerQueryResult> {
    let IpAddr::V4(v4) = addr.ip() else {
        return Err(AppError::invalid(
            "SA-MP servers are queried over IPv4 only.",
        ));
    };

    let mut request = Vec::with_capacity(HEADER_LEN);
    request.extend_from_slice(b"SAMP");
    request.extend_from_slice(&v4.octets());
    request.extend_from_slice(&addr.port().to_le_bytes());
    request.push(OPCODE_INFO);

    let (raw, rtt) = udp_exchange(addr, &request, timeout).await?;

    parse_info(&raw, addr.port(), rtt)
}

fn parse_info(raw: &[u8], port: u16, rtt: u32) -> AppResult<ServerQueryResult> {
    let mut r = Reader::new(raw);

    if r.bytes(4)? != b"SAMP" {
        return Err(AppError::invalid("Not a SA-MP reply."));
    }

    // The echoed address, port and opcode.
    r.skip(HEADER_LEN - 4)?;

    let mut out = ServerQueryResult::new(QueryProtocol::Samp, port, rtt);

    out.password = Some(r.u8()? != 0);
    out.players = Some(u32::from(r.u16_le()?));
    out.max_players = Some(u32::from(r.u16_le()?));

    // Length-prefixed with a 32-bit count. `Reader::pstring` bounds it against
    // both `MAX_STRING` and what is actually left in the buffer.
    out.name = Some(r.pstring(4)?);
    out.game = Some(r.pstring(4)?);

    // The language field is optional on some forks.
    if let Ok(language) = r.pstring(4) {
        if !language.is_empty() {
            out.rules.insert("language".into(), language);
        }
    }

    Ok(out)
}

/// The `r` opcode's rule list, kept for completeness — not requested by the
/// browser, since it is a second round trip for data no card renders.
#[allow(dead_code)]
fn parse_rules(raw: &[u8]) -> AppResult<Vec<(String, String)>> {
    let mut r = Reader::new(raw);

    if r.bytes(4)? != b"SAMP" {
        return Err(AppError::invalid("Not a SA-MP reply."));
    }

    r.skip(HEADER_LEN - 4)?;

    let count = usize::from(r.u16_le()?).min(128);
    let mut out = Vec::with_capacity(count.min(32));

    for _ in 0..count {
        if r.is_empty() {
            break;
        }

        let Ok(key) = r.pstring(1) else { break };
        let Ok(value) = r.pstring(1) else { break };

        out.push((key, value));
    }

    Ok(out)
}

/// SA-MP strings are Windows-1252 in practice, not UTF-8. The reader's lossy
/// decode turns high bytes into replacement characters; this is here so a
/// future fix has an obvious home, and is currently unused because the lossy
/// result is readable for the ASCII names that dominate.
#[allow(dead_code)]
fn decode_cp1252(raw: &[u8]) -> String {
    clean_text(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(name: &str, gamemode: &str, language: Option<&str>) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"SAMP");
        b.extend_from_slice(&[127, 0, 0, 1]);
        b.extend_from_slice(&7777u16.to_le_bytes());
        b.push(OPCODE_INFO);

        b.push(1); // password
        b.extend_from_slice(&42u16.to_le_bytes()); // players
        b.extend_from_slice(&100u16.to_le_bytes()); // max

        for field in [Some(name), Some(gamemode), language].into_iter().flatten() {
            b.extend_from_slice(&(field.len() as u32).to_le_bytes());
            b.extend_from_slice(field.as_bytes());
        }

        b
    }

    #[test]
    fn parses_an_info_reply() {
        let out = parse_info(
            &reply("Los Santos RP", "Roleplay", Some("English")),
            7777,
            60,
        )
        .expect("parses");

        assert!(out.online);
        assert_eq!(out.password, Some(true));
        assert_eq!(out.players, Some(42));
        assert_eq!(out.max_players, Some(100));
        assert_eq!(out.name.as_deref(), Some("Los Santos RP"));
        assert_eq!(out.game.as_deref(), Some("Roleplay"));
        assert_eq!(
            out.rules.get("language").map(String::as_str),
            Some("English")
        );
    }

    #[test]
    fn a_reply_without_the_optional_language_still_parses() {
        let out = parse_info(&reply("Server", "Freeroam", None), 7777, 1).expect("parses");

        assert_eq!(out.name.as_deref(), Some("Server"));
        assert!(!out.rules.contains_key("language"));
    }

    #[test]
    fn a_non_samp_reply_is_rejected() {
        assert!(parse_info(b"NOPE0000000", 7777, 0).is_err());
        assert!(parse_info(&[], 7777, 0).is_err());
    }

    #[test]
    fn a_lying_string_length_cannot_overread() {
        let mut b = Vec::new();
        b.extend_from_slice(b"SAMP");
        b.extend_from_slice(&[1, 2, 3, 4]);
        b.extend_from_slice(&7777u16.to_le_bytes());
        b.push(OPCODE_INFO);
        b.push(0);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        // Claims a 4 GB name.
        b.extend_from_slice(&u32::MAX.to_le_bytes());
        b.extend_from_slice(b"short");

        assert!(parse_info(&b, 7777, 0).is_err());
    }

    #[test]
    fn truncated_replies_never_panic() {
        let full = reply("Server", "Mode", Some("EN"));

        for cut in 0..full.len() {
            let _ = parse_info(&full[..cut], 7777, 0);
        }
    }

    #[test]
    fn parses_a_rule_list() {
        let mut b = Vec::new();
        b.extend_from_slice(b"SAMP");
        b.extend_from_slice(&[1, 2, 3, 4]);
        b.extend_from_slice(&7777u16.to_le_bytes());
        b.push(b'r');
        b.extend_from_slice(&2u16.to_le_bytes());

        for (k, v) in [("mapname", "San Andreas"), ("weather", "10")] {
            b.push(k.len() as u8);
            b.extend_from_slice(k.as_bytes());
            b.push(v.len() as u8);
            b.extend_from_slice(v.as_bytes());
        }

        let rules = parse_rules(&b).expect("parses");

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].0, "mapname");
        assert_eq!(rules[1].1, "10");
    }
}
