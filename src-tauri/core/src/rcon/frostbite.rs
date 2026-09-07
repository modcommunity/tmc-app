//! Frostbite RCON — Battlefield 3, 4, Hardline and Bad Company 2.
//!
//! Nothing like Valve's. A packet is a sequence header and a list of
//! length-prefixed words, and the query side of it is already implemented in
//! [`crate::net::query::frostbite`]; this is the half that logs in and runs
//! commands.
//!
//! ```text
//!   uint32  sequence   — bit 31: 0 from client, 1 from server
//!                        bit 30: 0 request,     1 response
//!   uint32  size       — the whole packet, header included
//!   uint32  numWords
//!   words              — each: uint32 length, bytes, NUL
//! ```
//!
//! LOGGING IN WITHOUT SENDING THE PASSWORD
//! ---------------------------------------
//! Two ways, and only one of them is acceptable:
//!
//!   * `login.plainText <password>` — sends it in the clear over a plaintext
//!     socket. **Not used here.**
//!   * `login.hashed` → the server returns a 16-byte salt as hex → the client
//!     answers `login.hashed MD5(salt || password)` in upper-case hex.
//!
//! MD5 is long dead as a hash and that is not this app's decision to revisit:
//! the protocol is fixed, the servers are shipped, and the alternative on offer
//! is sending the password itself. What the salted form does buy is real —
//! somebody watching the wire cannot replay the exchange against another
//! session — and it is strictly better than the option beside it.
//!
//! The socket is plaintext either way, which is a property of Frostbite RCON
//! and worth saying plainly rather than papering over: anyone on the path sees
//! every command and every reply. It is the same exposure the game's own tools
//! have.

use std::time::Duration;

use md5::{Digest, Md5};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{AppError, AppResult};
use crate::net::reader::Reader;

/// Cap on one packet.
const MAX_PACKET: usize = 1 << 20;

/// The two fixed header fields, which `size` counts.
const SEQUENCE_BYTES: usize = 4;
const SIZE_BYTES: usize = 4;

/// Cap on words in one reply.
const MAX_WORDS: usize = 8_192;

/// An authenticated connection.
pub struct FrostbiteRcon {
    stream: TcpStream,
    timeout: Duration,
    sequence: u32,
}

impl FrostbiteRcon {
    pub async fn connect(
        addr: std::net::SocketAddr,
        password: &str,
        timeout: Duration,
    ) -> AppResult<Self> {
        let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| AppError::Network("The server did not answer in time.".into()))?
            .map_err(|e| AppError::Network(format!("Could not connect: {e}")))?;

        let _ = stream.set_nodelay(true);

        let mut rcon = Self {
            stream,
            timeout,
            sequence: 0,
        };

        rcon.authenticate(password).await?;

        Ok(rcon)
    }

    async fn authenticate(&mut self, password: &str) -> AppResult<()> {
        let salt_reply = self.send(&["login.hashed"]).await?;

        let salt_hex = match salt_reply.first().map(String::as_str) {
            Some("OK") => salt_reply
                .get(1)
                .ok_or_else(|| AppError::Network("The server sent no login salt.".into()))?,
            Some(other) => {
                return Err(AppError::Network(format!(
                    "The server refused the login ({other})."
                )))
            }
            None => return Err(AppError::Network("The server sent an empty reply.".into())),
        };

        let salt = decode_hex(salt_hex)
            .ok_or_else(|| AppError::Network("The server sent an unreadable salt.".into()))?;

        let mut hasher = Md5::new();

        hasher.update(&salt);
        hasher.update(password.as_bytes());

        // Upper-case hex. Several server builds compare the string rather than
        // the bytes, and reject a lower-case digest that is otherwise correct.
        let digest = hex::encode_upper(hasher.finalize());

        let reply = self.send(&["login.hashed", &digest]).await?;

        match reply.first().map(String::as_str) {
            Some("OK") => Ok(()),
            // `InvalidPasswordHash` — a wrong password, and its own outcome
            // rather than a network failure, so the UI can say which it was.
            Some("InvalidPasswordHash") | Some("InvalidPassword") => Err(AppError::AuthRejected),
            Some(other) => Err(AppError::Network(format!(
                "The server refused the login ({other})."
            ))),
            None => Err(AppError::Network("The server sent an empty reply.".into())),
        }
    }

    /// Run one command. Frostbite commands are already a word list, so the
    /// caller's string is split on whitespace — which is what the game's own
    /// console does.
    pub async fn exec(&mut self, command: &str) -> AppResult<String> {
        let words: Vec<&str> = command.split_whitespace().collect();

        if words.is_empty() {
            return Err(AppError::invalid("Type a command."));
        }

        if words.len() > 64 {
            return Err(AppError::invalid("That command has too many arguments."));
        }

        let reply = self.send(&words).await?;

        /*
         * The first word is the status. `OK` is dropped from the output — it is
         * protocol noise, and leaving it means every successful command in the
         * console begins with a word the user did not ask for. Anything else is
         * kept, because it is the error.
         */
        let body = match reply.first().map(String::as_str) {
            Some("OK") => reply[1..].join(" "),
            _ => reply.join(" "),
        };

        Ok(body)
    }

    async fn send(&mut self, words: &[&str]) -> AppResult<Vec<String>> {
        self.sequence = self.sequence.wrapping_add(1) & 0x3fff_ffff;

        let packet = encode(self.sequence, words);

        tokio::time::timeout(self.timeout, self.stream.write_all(&packet))
            .await
            .map_err(|_| AppError::Network("The server stopped accepting data.".into()))?
            .map_err(|e| AppError::Network(format!("Could not send the command: {e}")))?;

        self.read_words().await
    }

    async fn read_words(&mut self) -> AppResult<Vec<String>> {
        let mut header = [0u8; 8];

        tokio::time::timeout(self.timeout, self.stream.read_exact(&mut header))
            .await
            .map_err(|_| AppError::Network("The server did not answer in time.".into()))?
            .map_err(|_| AppError::Network("The connection closed.".into()))?;

        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);

        // `size` counts the sequence and the size field, both of which are
        // already read.
        let remaining = size
            .checked_sub((SEQUENCE_BYTES + SIZE_BYTES) as u32)
            .ok_or_else(|| AppError::invalid("The server sent a truncated packet."))?
            as usize;

        if remaining > MAX_PACKET {
            return Err(AppError::invalid("The server sent an oversized packet."));
        }

        let mut body = vec![0u8; remaining];

        if remaining > 0 {
            tokio::time::timeout(self.timeout, self.stream.read_exact(&mut body))
                .await
                .map_err(|_| AppError::Network("The server stopped responding.".into()))?
                .map_err(|_| AppError::Network("The connection closed.".into()))?;
        }

        decode_words(&body)
    }
}

/// Build a request packet.
pub fn encode(sequence: u32, words: &[&str]) -> Vec<u8> {
    let mut body = Vec::new();

    body.extend_from_slice(&(words.len() as u32).to_le_bytes());

    for word in words {
        body.extend_from_slice(&(word.len() as u32).to_le_bytes());
        body.extend_from_slice(word.as_bytes());
        body.push(0);
    }

    /*
     * `size` is the WHOLE packet — the sequence and the size field included —
     * not the payload. Getting that wrong by the width of one field is the
     * classic Frostbite bug: the server reads four bytes of the next packet as
     * part of this one and every subsequent reply is off by four, which
     * presents as "the second command always fails".
     */
    let total = SEQUENCE_BYTES + SIZE_BYTES + body.len();

    let mut out = Vec::with_capacity(total);

    // Bit 31 clear (from client), bit 30 clear (a request).
    out.extend_from_slice(&(sequence & 0x3fff_ffff).to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&body);

    out
}

/// `numWords`, then that many length-prefixed, NUL-terminated words.
pub fn decode_words(body: &[u8]) -> AppResult<Vec<String>> {
    let mut reader = Reader::new(body);

    let count = reader.u32_le()? as usize;

    let mut words = Vec::with_capacity(count.min(64));

    for _ in 0..count.min(MAX_WORDS) {
        let Ok(word) = reader.pstring(4) else { break };

        words.push(word);

        // The NUL the length prefix did not count.
        if reader.u8().is_err() {
            break;
        }
    }

    Ok(words)
}

/// Hex → bytes, or `None`.
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || text.len() > 512 {
        return None;
    }

    hex::decode(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a reply the way a server would, so the decoder is tested against
    /// the shape it will actually meet.
    fn reply(words: &[&str]) -> Vec<u8> {
        let mut body = Vec::new();

        body.extend_from_slice(&(words.len() as u32).to_le_bytes());

        for word in words {
            body.extend_from_slice(&(word.len() as u32).to_le_bytes());
            body.extend_from_slice(word.as_bytes());
            body.push(0);
        }

        body
    }

    #[test]
    fn a_request_declares_its_own_true_length() {
        let raw = encode(1, &["login.hashed"]);

        let size = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]);

        assert_eq!(size as usize, raw.len());
    }

    #[test]
    fn a_request_round_trips_through_the_reply_decoder() {
        let raw = encode(7, &["admin.say", "hello", "all"]);

        // Everything after the 8-byte header is the word list.
        let words = decode_words(&raw[8..]).expect("decoded");

        assert_eq!(words, vec!["admin.say", "hello", "all"]);
    }

    #[test]
    fn the_sequence_never_sets_the_response_or_origin_bits() {
        // A packet claiming to be FROM the server, or to BE a response, is one
        // several server builds drop silently.
        for seq in [0u32, 1, 0x3fff_ffff, 0x4000_0000, 0x8000_0000, u32::MAX] {
            let raw = encode(seq, &["x"]);

            let written = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);

            assert_eq!(written & 0xc000_0000, 0, "seq {seq:x} set a reserved bit");
        }
    }

    #[test]
    fn a_login_salt_decodes_from_hex() {
        assert_eq!(decode_hex("00ff10"), Some(vec![0x00, 0xff, 0x10]));
        // Odd length, and non-hex.
        assert_eq!(decode_hex("abc"), None);
        assert_eq!(decode_hex("zz"), None);
    }

    /// The digest is `MD5(salt || password)` in UPPER-case hex. Several server
    /// builds compare the string, so a lower-case digest that is otherwise
    /// correct is rejected.
    #[test]
    fn the_login_digest_is_salt_then_password_in_upper_hex() {
        let salt = decode_hex("0011223344556677").expect("salt");

        let mut hasher = Md5::new();
        hasher.update(&salt);
        hasher.update(b"secret");

        let digest = hex::encode_upper(hasher.finalize());

        assert_eq!(digest.len(), 32);
        assert_eq!(digest, digest.to_uppercase());

        // Order matters: password-then-salt is a different, wrong answer.
        let mut wrong = Md5::new();
        wrong.update(b"secret");
        wrong.update(&salt);

        assert_ne!(digest, hex::encode_upper(wrong.finalize()));
    }

    #[test]
    fn every_truncation_of_a_reply_is_handled() {
        let raw = reply(&["OK", "hostname", "Test Server", "32", "64"]);

        for cut in 0..raw.len() {
            // A prefix too short to hold the word count is an ordinary error;
            // anything longer must decode to a partial list. Neither may panic,
            // which is the property being pinned.
            match decode_words(&raw[..cut]) {
                Ok(words) => assert!(words.len() <= 5),
                Err(_) => assert!(cut < 4, "cut {cut} should have decoded"),
            }
        }
    }

    /// The bug this pins cost the second command on every connection: a `size`
    /// that omits the two header fields leaves four bytes of the next packet in
    /// the buffer, and every reply after the first is misframed.
    #[test]
    fn the_size_field_counts_the_header_it_is_part_of() {
        for words in [vec!["x"], vec!["admin.say", "hello"], vec![]] {
            let raw = encode(1, &words);

            let size = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;

            assert_eq!(size, raw.len(), "{words:?}");

            // And what `read_words` would be left to read is exactly the body.
            assert_eq!(size - (SEQUENCE_BYTES + SIZE_BYTES), raw.len() - 8);
        }
    }

    #[test]
    fn an_absurd_word_count_does_not_allocate_against_it() {
        let mut body = u32::MAX.to_le_bytes().to_vec();

        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(b"hi\0");

        let words = decode_words(&body).expect("decoded");

        assert_eq!(words, vec!["hi"]);
    }

    #[test]
    fn garbage_never_panics() {
        for bad in [
            vec![],
            vec![0xff; 4],
            vec![0x00; 16],
            (0..255u8).collect::<Vec<u8>>(),
        ] {
            let _ = decode_words(&bad);
        }
    }
}
