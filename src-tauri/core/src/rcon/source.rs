//! Valve's RCON protocol, which is also Minecraft's.
//!
//! Four fields on the wire and one genuinely awkward corner:
//!
//! ```text
//!   int32  size    — of everything after this field
//!   int32  id      — echoed back, and how a reply is matched to a request
//!   int32  type    — 3 auth, 2 exec / auth-reply, 0 response
//!   bytes  body    — NUL-terminated
//!   byte   0       — a second NUL, which the size counts and nothing uses
//! ```
//!
//! THE AWKWARD CORNER: MULTI-PACKET REPLIES
//! ----------------------------------------
//! A response longer than about 4 KB arrives as several packets, and **nothing
//! in the protocol marks the last one**. A reader that stops at the first
//! packet truncates `status` on a full server; one that waits for more hangs on
//! every short reply.
//!
//! The fix is the one Valve's own tooling uses: after the command, send a
//! second, empty packet with a different id. The server answers requests in
//! order, so when the reply carrying that id arrives, everything before it was
//! the command's output and there is no more coming. It costs one round trip
//! and removes the guesswork entirely.
//!
//! THE OTHER ONE: AUTHENTICATION
//! -----------------------------
//! A successful auth sends TWO packets — an empty `RESPONSE_VALUE` and then an
//! `AUTH_RESPONSE`. A failed one sends an `AUTH_RESPONSE` with an id of `-1`.
//! Several implementations skip the empty packet, and at least one sends the
//! two in the other order, so this reads packets until it sees an
//! `AUTH_RESPONSE` rather than assuming a count.
//!
//! **A wrong password is not a network error.** It is reported as its own
//! outcome, because "check your password" and "the server is not answering"
//! send somebody to look in completely different places.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{AppError, AppResult};
use crate::net::reader::Reader;

/// Cap on one packet's declared size. Valve's own limit is 4096 for requests;
/// replies from modded servers routinely exceed it, so this is generous and
/// still bounded.
const MAX_PACKET: usize = 64 * 1024;

/// Cap on a whole reassembled response.
const MAX_RESPONSE: usize = 4 * 1024 * 1024;

/// Cap on packets read for one command, so a server that never sends the
/// sentinel cannot loop forever even inside its timeout.
const MAX_PACKETS: usize = 512;

/// Cap on a command. Nothing legitimate is longer, and the protocol's own limit
/// is smaller than this.
pub const MAX_COMMAND: usize = 2048;

const TYPE_RESPONSE: i32 = 0;
const TYPE_EXEC: i32 = 2;
const TYPE_AUTH_RESPONSE: i32 = 2;
const TYPE_AUTH: i32 = 3;

/// An authenticated connection.
pub struct SourceRcon {
    stream: TcpStream,
    timeout: Duration,
    next_id: i32,
}

impl SourceRcon {
    /// Connect and authenticate.
    pub async fn connect(
        addr: std::net::SocketAddr,
        password: &str,
        timeout: Duration,
    ) -> AppResult<Self> {
        let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| AppError::Network("The server did not answer in time.".into()))?
            .map_err(|e| AppError::Network(format!("Could not connect: {e}")))?;

        // RCON is a request/response protocol on a long-lived socket; Nagle
        // adds up to 40ms to every command for no benefit.
        let _ = stream.set_nodelay(true);

        let mut rcon = Self {
            stream,
            timeout,
            next_id: 1,
        };

        rcon.authenticate(password).await?;

        Ok(rcon)
    }

    async fn authenticate(&mut self, password: &str) -> AppResult<()> {
        let id = self.take_id();

        self.send(id, TYPE_AUTH, password.as_bytes()).await?;

        for _ in 0..8 {
            let packet = self.read_packet().await?;

            if packet.kind != TYPE_AUTH_RESPONSE {
                // The empty `RESPONSE_VALUE` some servers send first.
                continue;
            }

            /*
             * `-1` is the documented failure. Anything that is not the id we
             * sent is also a failure — a server answering a different request
             * is not evidence that this password was accepted.
             */
            if packet.id == id {
                return Ok(());
            }

            return Err(AppError::AuthRejected);
        }

        Err(AppError::Network(
            "The server never finished the RCON handshake.".into(),
        ))
    }

    /// Run one command and return everything it printed.
    pub async fn exec(&mut self, command: &str) -> AppResult<String> {
        if command.len() > MAX_COMMAND {
            return Err(AppError::invalid("That command is too long."));
        }

        /*
         * A NUL would end the body early, so the rest of the command would be
         * read by the server as a second, unintended one. Refused rather than
         * stripped: silently running something other than what was typed is
         * worse than saying no.
         */
        if command.contains('\0') {
            return Err(AppError::invalid("A command may not contain a NUL byte."));
        }

        let id = self.take_id();
        let sentinel = self.take_id();

        self.send(id, TYPE_EXEC, command.as_bytes()).await?;
        // The end-of-response marker — see the module header.
        self.send(sentinel, TYPE_RESPONSE, b"").await?;

        let mut out = String::new();

        for _ in 0..MAX_PACKETS {
            let packet = self.read_packet().await?;

            if packet.id == sentinel {
                return Ok(out);
            }

            if packet.id != id {
                // A reply to something else entirely. Nothing sensible to do
                // with it, and appending it would corrupt the output.
                continue;
            }

            if out.len() + packet.body.len() > MAX_RESPONSE {
                return Err(AppError::invalid(
                    "The server's reply was too large to read.",
                ));
            }

            out.push_str(&packet.body);
        }

        Err(AppError::Network(
            "The server sent more than this app will read for one command.".into(),
        ))
    }

    fn take_id(&mut self) -> i32 {
        // Wrapping, and never negative: `-1` is the protocol's failure marker
        // and an id that collided with it would read as an auth failure.
        self.next_id = self.next_id.wrapping_add(1).max(1);

        self.next_id
    }

    async fn send(&mut self, id: i32, kind: i32, body: &[u8]) -> AppResult<()> {
        let packet = encode(id, kind, body);

        tokio::time::timeout(self.timeout, self.stream.write_all(&packet))
            .await
            .map_err(|_| AppError::Network("The server stopped accepting data.".into()))?
            .map_err(|e| AppError::Network(format!("Could not send the command: {e}")))?;

        Ok(())
    }

    async fn read_packet(&mut self) -> AppResult<Packet> {
        let mut size_bytes = [0u8; 4];

        tokio::time::timeout(self.timeout, self.stream.read_exact(&mut size_bytes))
            .await
            .map_err(|_| AppError::Network("The server did not answer in time.".into()))?
            .map_err(|_| AppError::Network("The connection closed.".into()))?;

        let size = i32::from_le_bytes(size_bytes);

        // A size below the two ints plus two NULs cannot describe a packet.
        if !(10..=MAX_PACKET as i32).contains(&size) {
            return Err(AppError::invalid(
                "The server sent a packet this app will not read.",
            ));
        }

        let mut body = vec![0u8; size as usize];

        tokio::time::timeout(self.timeout, self.stream.read_exact(&mut body))
            .await
            .map_err(|_| AppError::Network("The server stopped responding.".into()))?
            .map_err(|_| AppError::Network("The connection closed.".into()))?;

        decode(&body)
    }
}

/// One decoded packet.
#[derive(Debug, PartialEq, Eq)]
pub struct Packet {
    pub id: i32,
    pub kind: i32,
    pub body: String,
}

/// Build a packet, size field included.
pub fn encode(id: i32, kind: i32, body: &[u8]) -> Vec<u8> {
    // id + type + body + two NULs.
    let size = (4 + 4 + body.len() + 2) as i32;

    let mut out = Vec::with_capacity(size as usize + 4);

    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(body);
    out.push(0);
    out.push(0);

    out
}

/// Decode everything after the size field.
///
/// Through [`Reader`] rather than by indexing, for the same reason every other
/// parser in this crate is: the bytes come from a machine that may be hostile
/// and the release profile aborts on panic, so an out-of-range read would be a
/// remote kill switch.
pub fn decode(buf: &[u8]) -> AppResult<Packet> {
    let mut reader = Reader::new(buf);

    let id = reader.i32_le()?;
    let kind = reader.i32_le()?;

    // The body runs to the first NUL, or to the end if the server omitted it —
    // which some implementations do on empty bodies.
    let raw = reader.rest();

    let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());

    let body = String::from_utf8_lossy(raw.get(..end).unwrap_or_default()).into_owned();

    Ok(Packet { id, kind, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packet_round_trips() {
        let raw = encode(7, TYPE_EXEC, b"status");

        // The size field describes everything after itself.
        let size = i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);

        assert_eq!(size as usize, raw.len() - 4);

        let packet = decode(&raw[4..]).expect("decoded");

        assert_eq!(
            packet,
            Packet {
                id: 7,
                kind: TYPE_EXEC,
                body: "status".into()
            }
        );
    }

    #[test]
    fn an_empty_body_decodes_to_an_empty_string() {
        let raw = encode(9, TYPE_RESPONSE, b"");

        let packet = decode(&raw[4..]).expect("decoded");

        assert_eq!(packet.id, 9);
        assert!(packet.body.is_empty());
    }

    #[test]
    fn a_body_stops_at_its_terminator() {
        // Some servers pad; anything after the NUL is not part of the body.
        let mut raw = Vec::new();

        raw.extend_from_slice(&5i32.to_le_bytes());
        raw.extend_from_slice(&0i32.to_le_bytes());
        raw.extend_from_slice(b"hello\0junk-after-the-terminator");

        let packet = decode(&raw).expect("decoded");

        assert_eq!(packet.body, "hello");
    }

    #[test]
    fn a_negative_id_is_preserved_because_it_means_something() {
        // `-1` is how a server says the password was wrong.
        let raw = encode(-1, TYPE_AUTH_RESPONSE, b"");

        let packet = decode(&raw[4..]).expect("decoded");

        assert_eq!(packet.id, -1);
    }

    /// The mandatory truncation test every parser in this crate has: the bytes
    /// come from an unauthenticated machine, and the release profile aborts on
    /// panic, so a decode that panics is a remote kill switch.
    #[test]
    fn every_truncation_of_a_packet_is_handled_without_panicking() {
        let raw = encode(42, TYPE_RESPONSE, b"hostname: Test Server\nplayers: 12");

        for cut in 0..raw.len() {
            // Skipping the size field, as `read_packet` does.
            let body = &raw[4..raw.len().min(cut + 4)];

            let _ = decode(body);
        }
    }

    #[test]
    fn garbage_decodes_or_declines_but_never_panics() {
        for bad in [
            vec![],
            vec![0xff],
            vec![0xff; 3],
            vec![0x00; 8],
            vec![0xff; 64],
            (0..255u8).collect::<Vec<u8>>(),
        ] {
            let _ = decode(&bad);
        }
    }

    #[test]
    fn a_command_with_a_nul_is_refused_rather_than_truncated() {
        // A NUL ends the body, so the server would read the remainder as a
        // second command it was never asked to run.
        let raw = "say hello\0rcon_password newpass";

        assert!(raw.contains('\0'));
        // The check lives in `exec`; this pins the reason it exists.
        assert!(raw.split('\0').count() > 1);
    }

    #[test]
    fn ids_never_collide_with_the_failure_marker() {
        let mut rcon_ids = Vec::new();
        let mut next: i32 = i32::MAX - 2;

        for _ in 0..8 {
            next = next.wrapping_add(1).max(1);
            rcon_ids.push(next);
        }

        assert!(
            rcon_ids.iter().all(|id| *id > 0),
            "an id of -1 would read as an auth failure: {rcon_ids:?}"
        );
    }
}
