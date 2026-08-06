//! Server Live Query plugins: send one datagram, read the reply with a
//! declarative field list.
//!
//! Different games speak entirely different query protocols — Source's A2S,
//! Minecraft's handshake, GameSpy's legacy format, a dozen bespoke ones — which
//! is exactly why this is a plugin type rather than a hardcoded list. But
//! "let the plugin parse it" cannot mean "let the plugin run a parser": a
//! parser driven by a remote server's bytes is where every memory-safety and
//! hang bug in this space lives.
//!
//! So the plugin supplies a *field list*, and this module walks the response
//! with a cursor that can only move forward inside a buffer the app allocated.
//! Every field type consumes a statically-known or length-prefixed number of
//! bytes, and running off the end is a clean error rather than a panic.

use serde_json::{Map, Value};

use crate::error::{AppError, AppResult};
use crate::net::addr::resolve_public;
use crate::net::transport::{clamp_timeout, udp_exchange};
use crate::plugins::manifest::{Field, PortSource, ServerQuery};

/// Cap on a decoded string field. Long enough for a MOTD, short enough that a
/// hostile server cannot make the UI allocate.
const MAX_STRING: usize = 1024;

pub async fn run(
    spec: &ServerQuery,
    host: &str,
    game_port: u16,
    query_port: Option<u16>,
) -> AppResult<Map<String, Value>> {
    let port = match spec.port {
        PortSource::Game => game_port,
        PortSource::Query => query_port.unwrap_or(game_port),
        PortSource::Fixed(p) => p,
    };

    if port == 0 {
        return Err(AppError::invalid("No port to query."));
    }

    let payload = hex::decode(spec.request_hex.trim())
        .map_err(|_| AppError::invalid("Query payload is not valid hex."))?;

    // Same public-address guard the built-in protocols use — a plugin's target
    // is no more trustworthy than a plugin's download host.
    let addr = resolve_public(host, port).await?;

    let (bytes, rtt_ms) = udp_exchange(addr, &payload, clamp_timeout(spec.timeout_ms)).await?;

    let mut out = parse(&spec.parse, &bytes)?;

    out.insert("_rttMs".into(), Value::from(rtt_ms));
    out.insert("_port".into(), Value::from(port));

    Ok(out)
}

/// Walk the field list over `bytes`.
///
/// `cursor` only ever increases, and every read checks the remaining length
/// first. There is no seek-backwards and no length taken from the response that
/// is not immediately bounds-checked against what is actually left.
pub fn parse(fields: &[Field], bytes: &[u8]) -> AppResult<Map<String, Value>> {
    let mut out = Map::new();
    let mut cursor = 0usize;

    let need = |cursor: usize, n: usize| -> AppResult<()> {
        if cursor.checked_add(n).is_none_or(|end| end > bytes.len()) {
            return Err(AppError::invalid(
                "Server response ended earlier than the plugin expected.",
            ));
        }

        Ok(())
    };

    for field in fields {
        match field {
            Field::Skip { bytes: n } => {
                let n = *n as usize;
                need(cursor, n)?;
                cursor += n;
            }

            Field::Magic { hex } => {
                let expected = hex::decode(hex.trim())
                    .map_err(|_| AppError::invalid("Query 'magic' is not valid hex."))?;

                need(cursor, expected.len())?;

                if &bytes[cursor..cursor + expected.len()] != expected.as_slice() {
                    return Err(AppError::invalid(
                        "Server response did not match the expected protocol.",
                    ));
                }

                cursor += expected.len();
            }

            Field::U8 { name } => {
                need(cursor, 1)?;
                out.insert(name.clone(), Value::from(bytes[cursor]));
                cursor += 1;
            }

            Field::U16Le { name } | Field::U16Be { name } => {
                need(cursor, 2)?;

                let raw = [bytes[cursor], bytes[cursor + 1]];
                let v = if matches!(field, Field::U16Le { .. }) {
                    u16::from_le_bytes(raw)
                } else {
                    u16::from_be_bytes(raw)
                };

                out.insert(name.clone(), Value::from(v));
                cursor += 2;
            }

            Field::U32Le { name } | Field::U32Be { name } => {
                need(cursor, 4)?;

                let mut raw = [0u8; 4];
                raw.copy_from_slice(&bytes[cursor..cursor + 4]);

                let v = if matches!(field, Field::U32Le { .. }) {
                    u32::from_le_bytes(raw)
                } else {
                    u32::from_be_bytes(raw)
                };

                out.insert(name.clone(), Value::from(v));
                cursor += 4;
            }

            Field::CString { name } => {
                let end = bytes[cursor..]
                    .iter()
                    .position(|b| *b == 0)
                    .map(|off| cursor + off)
                    // An unterminated string is a malformed response, not an
                    // invitation to read to the end of the buffer.
                    .ok_or_else(|| {
                        AppError::invalid("Server response had an unterminated string.")
                    })?;

                if end - cursor > MAX_STRING {
                    return Err(AppError::invalid("Server response string is too long."));
                }

                out.insert(name.clone(), Value::from(clean(&bytes[cursor..end])));
                cursor = end + 1;
            }

            Field::PString { name } => {
                need(cursor, 1)?;

                let len = bytes[cursor] as usize;
                cursor += 1;

                need(cursor, len)?;

                if len > MAX_STRING {
                    return Err(AppError::invalid("Server response string is too long."));
                }

                out.insert(
                    name.clone(),
                    Value::from(clean(&bytes[cursor..cursor + len])),
                );
                cursor += len;
            }
        }
    }

    Ok(out)
}

/// Decode lossily and strip control characters.
///
/// Game servers routinely put colour codes, ANSI escapes and raw bytes in their
/// hostname, and this string ends up rendered in the UI. Lossy UTF-8 handles
/// the invalid sequences; dropping controls handles the rest.
fn clean(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_STRING)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_source_style_reply() {
        // 0xFFFFFFFF header, 'I', protocol byte, name, map.
        let mut bytes = vec![0xFF, 0xFF, 0xFF, 0xFF, b'I', 17];
        bytes.extend_from_slice(b"Test Server\0");
        bytes.extend_from_slice(b"de_dust2\0");

        let fields = vec![
            Field::Magic {
                hex: "ffffffff".into(),
            },
            Field::Skip { bytes: 1 },
            Field::U8 {
                name: "protocol".into(),
            },
            Field::CString {
                name: "name".into(),
            },
            Field::CString { name: "map".into() },
        ];

        let out = parse(&fields, &bytes).expect("parses");

        assert_eq!(out["protocol"], Value::from(17u8));
        assert_eq!(out["name"], Value::from("Test Server"));
        assert_eq!(out["map"], Value::from("de_dust2"));
    }

    #[test]
    fn a_truncated_response_errors_rather_than_panicking() {
        let fields = vec![Field::U32Le { name: "x".into() }];

        assert!(parse(&fields, &[1, 2]).is_err());
    }

    #[test]
    fn an_unterminated_string_is_rejected() {
        let fields = vec![Field::CString { name: "x".into() }];

        assert!(parse(&fields, b"no terminator").is_err());
    }

    #[test]
    fn a_lying_length_prefix_cannot_overread() {
        // Claims 200 bytes follow; only 3 do.
        let fields = vec![Field::PString { name: "x".into() }];

        assert!(parse(&fields, &[200, b'a', b'b', b'c']).is_err());
    }

    #[test]
    fn control_characters_are_stripped_from_strings() {
        let mut bytes = b"na\x1b[31mme".to_vec();
        bytes.push(0);

        let out = parse(&[Field::CString { name: "n".into() }], &bytes).expect("parses");

        assert_eq!(out["n"], Value::from("na[31mme"));
    }
}
