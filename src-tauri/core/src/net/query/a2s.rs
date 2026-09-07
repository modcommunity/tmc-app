//! Valve's A2S — the protocol behind CS2, TF2, Rust, ARK, Garry's Mod, Squad,
//! and most of the Source/GoldSrc catalogue.
//!
//! Two things make this more than a request/response:
//!
//!   * **A challenge handshake.** Since the 2015 amplification-attack patch,
//!     servers answer the first `A2S_INFO` with `0x41` + a four-byte challenge
//!     rather than the data; the query is re-sent with the challenge appended.
//!     That is a reflection-DDoS mitigation, and it means any implementation
//!     that does not handle `0x41` sees every modern server as unreachable.
//!   * **Split replies.** `A2S_PLAYER` on a full server exceeds an MTU and
//!     comes back as `0xFFFFFFFE` fragments, pushed without further prompting
//!     and reassembled here by each fragment's own index.
//!
//! Both need the SAME socket for the whole conversation — see the note in
//! [`query`] — and both are bounded: one challenge retry, never a loop, and a
//! fixed fragment cap plus the caller's deadline.

use std::net::SocketAddr;
use std::time::Duration;

use crate::error::{AppError, AppResult};
use crate::net::query::{PlayerEntry, QueryProtocol, ServerQueryResult};
use crate::net::reader::{strip_colour_codes, Reader};
use crate::net::transport::UdpSession;

const SINGLE: i32 = -1; // 0xFFFFFFFF
const SPLIT: i32 = -2; // 0xFFFFFFFE

const HEADER_INFO: u8 = b'T';
const HEADER_PLAYER: u8 = b'U';
const REPLY_INFO: u8 = b'I';
const REPLY_INFO_GOLDSRC: u8 = b'm';
const REPLY_PLAYER: u8 = b'D';
const REPLY_CHALLENGE: u8 = b'A';

const A2S_INFO_PAYLOAD: &[u8] = b"\xFF\xFF\xFF\xFFTSource Engine Query\0";

/// Fragments accepted for one split reply.
///
/// The count is a whole byte, so a server may legitimately announce up to 255;
/// a 256-slot roster arrives in a handful. The cap is what stops a hostile
/// reply making this allocate 255 slots and then wait out the deadline for
/// fragments that will never come.
const MAX_FRAGMENTS: usize = 32;

/// Bit in the reply id meaning "the joined payload is bzip2".
const COMPRESSED: u32 = 0x8000_0000;

/// Cap on what a compressed reply may claim to decompress to.
///
/// A bzip2 bomb is a few hundred bytes that expands without limit, and this
/// arrives from an unauthenticated machine over UDP. `go-a2s` — which is what
/// `spy` itself uses — caps at exactly 1 MB, and a roster does not approach it.
const MAX_DECOMPRESSED: usize = 1024 * 1024;

/// Roster entries kept. Above this the list is not useful to a human anyway,
/// and it bounds what a lying server can make the UI render.
const MAX_PLAYERS: usize = 256;

pub async fn query(
    addr: SocketAddr,
    timeout: Duration,
    want_players: bool,
) -> AppResult<ServerQueryResult> {
    /*
     * ONE socket for the whole conversation.
     *
     * Valve issues its challenge against the querying address AND port, so a
     * challenged retry sent from a fresh ephemeral port is ignored by several
     * server builds — which presents as "this server never answers" rather than
     * as anything diagnosable. Reusing the session is also what makes a split
     * reply readable at all: the continuation datagrams arrive unprompted on
     * the socket that asked.
     */
    let session = UdpSession::connect(addr).await?;

    let (info_bytes, rtt) =
        request_with_challenge(&session, timeout, A2S_INFO_PAYLOAD, HEADER_INFO).await?;

    let mut result = parse_info(&info_bytes, addr.port(), rtt)?;

    if want_players {
        // A failed roster must not fail the whole query — plenty of servers
        // answer A2S_INFO and refuse A2S_PLAYER.
        if let Ok(players) = fetch_players(&session, timeout).await {
            result.player_list = players;
        }
    }

    Ok(result)
}

/// Send, and re-send once if the server answers with a challenge.
async fn request_with_challenge(
    session: &UdpSession,
    timeout: Duration,
    payload: &[u8],
    header: u8,
) -> AppResult<(Vec<u8>, u32)> {
    let (first, rtt) = session.exchange(payload, timeout).await?;

    let Some(kind) = reply_kind(&first)? else {
        return Ok((first, rtt));
    };

    if kind != REPLY_CHALLENGE {
        return Ok((first, rtt));
    }

    // `0x41` + four challenge bytes. Append them and ask once more.
    let mut r = Reader::new(&first);
    r.skip(4)?; // -1 single-packet header
    r.skip(1)?; // 'A'

    let challenge = r.bytes(4)?;

    let mut retry = Vec::with_capacity(payload.len() + 4);

    if header == HEADER_INFO {
        // A2S_INFO carries the challenge after the payload string.
        retry.extend_from_slice(payload);
        retry.extend_from_slice(challenge);
    } else {
        // A2S_PLAYER replaces its placeholder challenge entirely.
        retry.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, header]);
        retry.extend_from_slice(challenge);
    }

    // ONE retry. A server that answers a challenged request with another
    // challenge is either broken or trying to keep us talking.
    let (second, rtt2) = session.exchange(&retry, timeout).await?;

    Ok((second, rtt.max(rtt2)))
}

/// The reply's type byte, or `None` when it is a split reply we must reassemble.
fn reply_kind(buf: &[u8]) -> AppResult<Option<u8>> {
    let mut r = Reader::new(buf);

    match r.i32_le()? {
        SINGLE => Ok(Some(r.u8()?)),
        SPLIT => Ok(None),
        _ => Err(AppError::invalid("Not an A2S reply.")),
    }
}

fn parse_info(buf: &[u8], port: u16, rtt: u32) -> AppResult<ServerQueryResult> {
    let mut r = Reader::new(buf);

    if r.i32_le()? != SINGLE {
        return Err(AppError::invalid("Not an A2S reply."));
    }

    let kind = r.u8()?;

    let mut out = ServerQueryResult::new(QueryProtocol::A2S, port, rtt);

    if kind == REPLY_INFO_GOLDSRC {
        // The pre-2010 GoldSrc layout: address string first, then name.
        let _address = r.cstring()?;
        out.name = Some(strip_colour_codes(&r.cstring()?));
        out.map = Some(r.cstring()?);
        let _folder = r.cstring()?;
        out.game = Some(r.cstring()?);
        out.players = Some(u32::from(r.u8()?));
        out.max_players = Some(u32::from(r.u8()?));
        out.version = Some(r.u8()?.to_string());

        return Ok(out);
    }

    if kind != REPLY_INFO {
        return Err(AppError::invalid("Unexpected A2S reply type."));
    }

    let _protocol = r.u8()?;

    out.name = Some(strip_colour_codes(&r.cstring()?));
    out.map = Some(r.cstring()?);
    let _folder = r.cstring()?;
    out.game = Some(r.cstring()?);
    let _app_id = r.u16_le()?;

    out.players = Some(u32::from(r.u8()?));
    out.max_players = Some(u32::from(r.u8()?));
    out.bots = Some(u32::from(r.u8()?));

    let _server_type = r.u8()?;
    let _environment = r.u8()?;

    out.password = Some(r.u8()? != 0);
    out.secure = Some(r.u8()? != 0);

    out.version = Some(r.cstring()?);

    /*
     * The extra-data block is optional and its presence is a bitfield. Every
     * read below is behind its flag AND behind `remaining()`, because a server
     * can set a flag and then not send the field — several modded ones do.
     */
    if r.remaining() >= 1 {
        let flags = r.u8()?;

        if flags & 0x80 != 0 && r.remaining() >= 2 {
            out.rules.insert("gamePort".into(), r.u16_le()?.to_string());
        }

        if flags & 0x10 != 0 && r.remaining() >= 8 {
            out.rules.insert("steamId".into(), r.u64_le()?.to_string());
        }

        if flags & 0x40 != 0 && r.remaining() >= 3 {
            let tv_port = r.u16_le()?;
            let tv_name = r.cstring().unwrap_or_default();

            out.rules.insert("tvPort".into(), tv_port.to_string());
            out.rules.insert("tvName".into(), tv_name);
        }

        if flags & 0x20 != 0 && r.remaining() >= 1 {
            let tags = r.cstring().unwrap_or_default();

            if !tags.is_empty() {
                out.rules.insert("tags".into(), tags);
            }
        }
    }

    Ok(out)
}

async fn fetch_players(session: &UdpSession, timeout: Duration) -> AppResult<Vec<PlayerEntry>> {
    // The placeholder challenge; the server answers 0x41 with the real one.
    let payload = [
        0xFF,
        0xFF,
        0xFF,
        0xFF,
        HEADER_PLAYER,
        0xFF,
        0xFF,
        0xFF,
        0xFF,
    ];

    let (bytes, _) = request_with_challenge(session, timeout, &payload, HEADER_PLAYER).await?;

    let assembled = if reply_kind(&bytes)?.is_none() {
        reassemble(session, timeout, bytes).await?
    } else {
        bytes
    };

    parse_players(&assembled)
}

/// Collect the remaining fragments of a split reply and join them in order.
///
/// A full server's roster exceeds an MTU and arrives as `0xFFFFFFFE` fragments
/// that the server pushes without further prompting — which is why this needs
/// the same session the request went out on.
///
/// Ordering is by each fragment's own index rather than by arrival, because UDP
/// promises nothing and a roster reassembled out of sequence is not a slightly
/// wrong roster, it is garbage. Bounded by `MAX_FRAGMENTS` and by the caller's
/// deadline: a server that announces twelve fragments and sends one must time
/// out, not wait forever.
async fn reassemble(session: &UdpSession, timeout: Duration, first: Vec<u8>) -> AppResult<Vec<u8>> {
    let head = split_header(&first)?;

    if head.total == 0 || head.total > MAX_FRAGMENTS {
        return Err(AppError::invalid("The server sent too many fragments."));
    }

    let total = head.total;
    let id = head.id;
    let compressed = head.compressed();

    let mut parts: Vec<Option<Vec<u8>>> = vec![None; total];

    let store = |parts: &mut Vec<Option<Vec<u8>>>, packet: Vec<u8>| -> AppResult<()> {
        let head = split_header(&packet)?;

        /*
         * Every fragment of one reply must agree about the id and the count.
         * The id check is not ceremony: a server answering two of our queries
         * at once puts both replies on this socket, and joining a fragment of
         * one with fragments of the other produces a roster made of two
         * different rosters' bytes.
         */
        if head.total != total || head.id != id || head.index >= total {
            return Err(AppError::invalid("The server sent inconsistent fragments."));
        }

        let Some(slot) = parts.get_mut(head.index) else {
            return Err(AppError::invalid("The server sent inconsistent fragments."));
        };

        // A repeat is a misparse or a hostile sender, not a retransmission to
        // be tolerated: taking the second copy is how a garbled roster gets
        // built out of packets that individually looked fine.
        if slot.is_some() {
            return Err(AppError::invalid("The server sent a fragment twice."));
        }

        *slot = Some(packet.get(head.offset..).unwrap_or(&[]).to_vec());

        Ok(())
    };

    store(&mut parts, first)?;

    let deadline = tokio::time::Instant::now() + timeout;

    while parts.iter().any(Option::is_none) {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());

        if remaining.is_zero() {
            break;
        }

        match session.recv_more(remaining).await {
            Ok(packet) => store(&mut parts, packet)?,
            Err(_) => break,
        }
    }

    let mut out = Vec::new();

    for part in parts {
        // A gap means the reply is incomplete; concatenating around it would
        // produce a plausible-looking roster made of misaligned bytes.
        let Some(body) = part else {
            return Err(AppError::invalid(
                "The server's player list arrived incomplete.",
            ));
        };

        out.extend_from_slice(&body);
    }

    if compressed {
        return decompress(&out);
    }

    Ok(out)
}

/// Un-bzip2 a joined split reply.
///
/// The compressed form puts two extra fields at the front of the JOINED
/// payload rather than in each fragment's header — the decompressed size and a
/// CRC32 of the decompressed bytes — which is why this happens after the join
/// and not during it.
///
/// Old mods only, in practice: no current Source server compresses. It is
/// implemented because the alternative is that those servers' rosters fail with
/// a parse error that looks exactly like the server being broken.
fn decompress(payload: &[u8]) -> AppResult<Vec<u8>> {
    use std::io::Read;

    let mut r = Reader::new(payload);

    let declared = r.u32_le()? as usize;
    let expected_crc = r.u32_le()?;

    /*
     * The size is checked BEFORE decompressing, and against a fixed cap rather
     * than against anything the reply also controls. bzip2 is a compression
     * bomb's favourite format — a few hundred bytes expand without limit — and
     * this arrives over UDP from an unauthenticated machine.
     */
    if declared == 0 || declared > MAX_DECOMPRESSED {
        return Err(AppError::invalid(
            "The server's compressed reply declares an implausible size.",
        ));
    }

    let body = payload.get(r.position()..).unwrap_or(&[]);

    let mut out = Vec::with_capacity(declared);

    /*
     * `take(declared)` bounds the reader itself, so the cap holds even if the
     * declared size is a lie — a stream that expands past it stops there and
     * fails the length check below rather than filling memory first.
     */
    bzip2::read::BzDecoder::new(body)
        .take(declared as u64)
        .read_to_end(&mut out)
        .map_err(|_| AppError::invalid("The server's compressed reply did not decompress."))?;

    if out.len() != declared {
        return Err(AppError::invalid(
            "The server's compressed reply was the wrong length.",
        ));
    }

    /*
     * The checksum is verified rather than trusted. A corrupted fragment
     * reassembled in the right order still decompresses to something, and
     * "something" rendered as a player list is worse than an error.
     */
    if crc32fast::hash(&out) != expected_crc {
        return Err(AppError::invalid(
            "The server's compressed reply failed its checksum.",
        ));
    }

    Ok(out)
}

/// One fragment's header.
struct SplitHeader {
    /// Shared by every fragment of one reply. Its top bit means the JOINED
    /// payload is bzip2, not this fragment.
    id: u32,
    total: usize,
    index: usize,
    /// Where this fragment's payload starts.
    offset: usize,
}

impl SplitHeader {
    fn compressed(&self) -> bool {
        self.id & COMPRESSED != 0
    }
}

/// Parse a fragment header.
///
/// Layout, after the `-2`: a 4-byte reply id, **`total` as a whole byte**,
/// **`number` as a whole byte**, then a 2-byte split size.
///
/// The two counts being separate bytes is the part that is easy to get wrong,
/// and this got it wrong: GoldSrc packs them into one byte (index in the high
/// nibble, total in the low) and has no size field at all, and reading that
/// layout while ALSO consuming a size field produced a parser that no server
/// matches. It looked correct because fragment 0 of a real Source reply decodes
/// to `total = 2, index = 0` under the packed reading — plausible — while every
/// later fragment decodes to index 0 as well, so they overwrote each other and
/// the reply always came out "incomplete". Which is to say: every split roster
/// on every full server failed, and the unit test agreed with the bug because
/// it built packets in the same invented shape.
///
/// The layout here is the one `go-a2s` implements — the library `spy` itself
/// queries A2S with — and it matches PHP-Source-Query and python-a2s. GoldSrc's
/// packed variant is not supported and is not worth supporting: every A2S game
/// in this catalogue is Orange Box or newer, and guessing between two layouts
/// that both parse produces a garbled roster rather than an error.
fn split_header(buf: &[u8]) -> AppResult<SplitHeader> {
    let mut r = Reader::new(buf);

    if r.i32_le()? != SPLIT {
        return Err(AppError::invalid("Not a split A2S reply."));
    }

    let id = r.u32_le()?;
    let total = usize::from(r.u8()?);
    let index = usize::from(r.u8()?);

    // The maximum-packet-size field. Present on Orange Box and later, which is
    // everything this speaks to; read and discarded.
    let _split_size = r.u16_le()?;

    Ok(SplitHeader {
        id,
        total,
        index,
        offset: r.position(),
    })
}

fn parse_players(buf: &[u8]) -> AppResult<Vec<PlayerEntry>> {
    let mut r = Reader::new(buf);

    if r.i32_le()? != SINGLE {
        return Err(AppError::invalid("Not an A2S reply."));
    }

    if r.u8()? != REPLY_PLAYER {
        return Err(AppError::invalid("Unexpected A2S reply type."));
    }

    let count = usize::from(r.u8()?).min(MAX_PLAYERS);

    let mut out = Vec::with_capacity(count.min(64));

    for _ in 0..count {
        // A server may claim more players than it sends. Stop cleanly.
        if r.remaining() < 3 {
            break;
        }

        let _index = r.u8()?;
        let name = strip_colour_codes(&r.cstring()?);
        let score = r.i32_le()?;
        let duration = r.f32_le()?;

        out.push(PlayerEntry {
            name,
            score: Some(score),
            // NaN and negatives come back from modded servers; clamp rather
            // than rendering "-1s connected".
            duration: (duration.is_finite() && duration >= 0.0).then_some(duration as u32),
            ping: None,
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info_reply() -> Vec<u8> {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF, REPLY_INFO, 17];
        b.extend_from_slice(b"^1Test ^7Server\0");
        b.extend_from_slice(b"de_dust2\0");
        b.extend_from_slice(b"cstrike\0");
        b.extend_from_slice(b"Counter-Strike\0");
        b.extend_from_slice(&240u16.to_le_bytes());
        b.extend_from_slice(&[12, 32, 2]); // players, max, bots
        b.extend_from_slice(b"dl"); // type, environment
        b.extend_from_slice(&[0, 1]); // password, secure
        b.extend_from_slice(b"1.0.0.1\0");
        b
    }

    #[test]
    fn parses_a_source_info_reply() {
        let out = parse_info(&info_reply(), 27015, 42).expect("parses");

        assert!(out.online);
        assert_eq!(out.rtt_ms, 42);
        assert_eq!(out.name.as_deref(), Some("Test Server"));
        assert_eq!(out.map.as_deref(), Some("de_dust2"));
        assert_eq!(out.game.as_deref(), Some("Counter-Strike"));
        assert_eq!(out.players, Some(12));
        assert_eq!(out.max_players, Some(32));
        assert_eq!(out.bots, Some(2));
        assert_eq!(out.password, Some(false));
        assert_eq!(out.secure, Some(true));
        assert_eq!(out.version.as_deref(), Some("1.0.0.1"));
    }

    #[test]
    fn parses_the_optional_extra_data_block() {
        let mut b = info_reply();
        b.push(0x80 | 0x20); // game port + tags
        b.extend_from_slice(&27015u16.to_le_bytes());
        b.extend_from_slice(b"comp,eu\0");

        let out = parse_info(&b, 27015, 1).expect("parses");

        assert_eq!(out.rules.get("gamePort").map(String::as_str), Some("27015"));
        assert_eq!(out.rules.get("tags").map(String::as_str), Some("comp,eu"));
    }

    #[test]
    fn a_flag_set_with_no_field_behind_it_does_not_fail_the_parse() {
        let mut b = info_reply();
        b.push(0x80); // claims a game port, sends nothing

        let out = parse_info(&b, 27015, 1).expect("still parses");

        assert_eq!(out.players, Some(12));
        assert!(!out.rules.contains_key("gamePort"));
    }

    #[test]
    fn truncated_replies_error_rather_than_panic() {
        let full = info_reply();

        for cut in 0..full.len() {
            // Every prefix must either parse or error — never panic.
            let _ = parse_info(&full[..cut], 27015, 0);
        }
    }

    #[test]
    fn a_goldsrc_reply_parses_through_the_other_branch() {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF, REPLY_INFO_GOLDSRC];
        b.extend_from_slice(b"1.2.3.4:27015\0");
        b.extend_from_slice(b"Old Server\0");
        b.extend_from_slice(b"crossfire\0");
        b.extend_from_slice(b"valve\0");
        b.extend_from_slice(b"Half-Life\0");
        b.extend_from_slice(&[8, 16, 47]);

        let out = parse_info(&b, 27015, 5).expect("parses");

        assert_eq!(out.name.as_deref(), Some("Old Server"));
        assert_eq!(out.map.as_deref(), Some("crossfire"));
        assert_eq!(out.players, Some(8));
    }

    #[test]
    fn parses_a_player_roster() {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF, REPLY_PLAYER, 2];

        b.push(0);
        b.extend_from_slice(b"alice\0");
        b.extend_from_slice(&15i32.to_le_bytes());
        b.extend_from_slice(&120.5f32.to_le_bytes());

        b.push(1);
        b.extend_from_slice(b"^3bob\0");
        b.extend_from_slice(&(-1i32).to_le_bytes());
        b.extend_from_slice(&f32::NAN.to_le_bytes());

        let players = parse_players(&b).expect("parses");

        assert_eq!(players.len(), 2);
        assert_eq!(players[0].name, "alice");
        assert_eq!(players[0].duration, Some(120));
        assert_eq!(players[1].name, "bob");
        // NaN must not become a nonsense duration.
        assert_eq!(players[1].duration, None);
    }

    #[test]
    fn a_roster_claiming_more_players_than_it_sends_stops_cleanly() {
        let mut b = vec![0xFF, 0xFF, 0xFF, 0xFF, REPLY_PLAYER, 200];

        b.push(0);
        b.extend_from_slice(b"only-one\0");
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&1.0f32.to_le_bytes());

        let players = parse_players(&b).expect("parses");

        assert_eq!(players.len(), 1);
    }

    #[test]
    fn a_non_a2s_reply_is_rejected() {
        assert!(parse_info(&[0, 0, 0, 0, b'I'], 1, 0).is_err());
        assert!(parse_info(&[0xFF, 0xFF, 0xFF, 0xFF, b'Z'], 1, 0).is_err());
    }

    /// One fragment, in the shape a real Orange Box server sends: `total` and
    /// `number` are SEPARATE bytes, then a split size.
    fn split_packet(id: u32, total: u8, index: u8, body: &[u8]) -> Vec<u8> {
        let mut b = vec![0xFE, 0xFF, 0xFF, 0xFF];

        b.extend_from_slice(&id.to_le_bytes());
        b.push(total);
        b.push(index);
        b.extend_from_slice(&1248u16.to_le_bytes()); // split size
        b.extend_from_slice(body);
        b
    }

    #[test]
    fn a_split_header_yields_count_index_and_payload_offset() {
        let packet = split_packet(7, 3, 2, b"body");
        let head = split_header(&packet).expect("parses");

        assert_eq!(head.total, 3);
        assert_eq!(head.index, 2);
        assert_eq!(head.id, 7);
        assert!(!head.compressed());
        assert_eq!(&packet[head.offset..], b"body");
    }

    /// The bug this layout replaced, stated as a test so it cannot come back.
    ///
    /// Under the old packed-nibble reading, EVERY fragment of a real reply
    /// decoded to index 0 — so they overwrote each other and the roster always
    /// came out incomplete. Fragment 0 decoded plausibly, which is why it went
    /// unnoticed.
    #[test]
    fn later_fragments_have_distinct_indices() {
        let indices: Vec<usize> = (0..4u8)
            .map(|n| {
                split_header(&split_packet(7, 4, n, b"x"))
                    .expect("parses")
                    .index
            })
            .collect();

        assert_eq!(indices, vec![0, 1, 2, 3]);
    }

    /// The top bit of the reply id, and nothing else, means bzip2.
    #[test]
    fn the_compression_bit_is_read_from_the_reply_id() {
        assert!(split_header(&split_packet(0x8000_0001, 2, 0, b"x"))
            .expect("parses")
            .compressed());

        assert!(!split_header(&split_packet(0x7FFF_FFFF, 2, 0, b"x"))
            .expect("parses")
            .compressed());
    }

    #[test]
    fn a_single_packet_is_not_a_split_header() {
        assert!(split_header(&[0xFF, 0xFF, 0xFF, 0xFF, b'D']).is_err());
        assert!(split_header(&[]).is_err());
    }

    #[test]
    fn truncated_split_headers_never_panic() {
        let full = split_packet(7, 2, 0, b"x");

        for cut in 0..full.len() {
            let _ = split_header(&full[..cut]);
        }
    }

    // ------------------------------------------------- Compressed replies

    fn compress(body: &[u8]) -> Vec<u8> {
        use std::io::Read;

        let mut out = Vec::new();

        bzip2::read::BzEncoder::new(body, bzip2::Compression::best())
            .read_to_end(&mut out)
            .expect("compress");

        out
    }

    /// A joined compressed payload: size, CRC32, then the bzip2 stream.
    fn compressed_payload(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();

        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&crc32fast::hash(body).to_le_bytes());
        out.extend_from_slice(&compress(body));

        out
    }

    #[test]
    fn a_compressed_reply_round_trips() {
        let body = b"\xFF\xFF\xFF\xFFD\x02player one\0";

        assert_eq!(
            decompress(&compressed_payload(body)).expect("decompresses"),
            body.to_vec()
        );
    }

    /// The checksum is verified rather than trusted: a reply reassembled out of
    /// the wrong fragments still decompresses to SOMETHING, and something
    /// rendered as a player list is worse than an error.
    #[test]
    fn a_compressed_reply_with_a_bad_checksum_is_refused() {
        let mut payload = compressed_payload(b"hello world");

        payload[4] ^= 0xFF;

        assert!(decompress(&payload).is_err());
    }

    #[test]
    fn a_compressed_reply_that_lies_about_its_length_is_refused() {
        let body = b"hello world";

        let mut payload = Vec::new();

        payload.extend_from_slice(&(body.len() as u32 + 5).to_le_bytes());
        payload.extend_from_slice(&crc32fast::hash(body).to_le_bytes());
        payload.extend_from_slice(&compress(body));

        assert!(decompress(&payload).is_err());
    }

    /// A bzip2 bomb is a few hundred bytes that expands without limit, arriving
    /// over UDP from an unauthenticated machine. The declared size is checked
    /// against a FIXED cap before anything is decompressed, and the reader is
    /// bounded as well so a lying size cannot get past it either.
    #[test]
    fn a_compression_bomb_is_refused_before_it_is_decompressed() {
        let mut payload = Vec::new();

        payload.extend_from_slice(&u32::MAX.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&compress(&vec![b'A'; 4096]));

        assert!(decompress(&payload).is_err());

        // And a size just over the cap, which is the near-miss version.
        let mut payload = Vec::new();

        payload.extend_from_slice(&((MAX_DECOMPRESSED + 1) as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&compress(&vec![b'A'; 4096]));

        assert!(decompress(&payload).is_err());
    }

    #[test]
    fn a_truncated_compressed_reply_never_panics() {
        let full = compressed_payload(b"a roster of some length, compressed");

        for cut in 0..=full.len() {
            let _ = decompress(&full[..cut]);
        }
    }

    #[test]
    fn garbage_where_a_bzip2_stream_should_be_is_an_error() {
        let mut payload = Vec::new();

        payload.extend_from_slice(&16u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(b"not a bzip2 stream at all");

        assert!(decompress(&payload).is_err());
    }

    #[test]
    fn reply_kind_recognises_a_split_header() {
        assert_eq!(
            reply_kind(&[0xFF, 0xFF, 0xFF, 0xFF, b'I']).expect("ok"),
            Some(b'I')
        );
        assert_eq!(
            reply_kind(&[0xFE, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 2]).expect("ok"),
            None
        );
    }
}
