//! The two transports every query protocol is built on.
//!
//! Both are hard-bounded. That is the whole design constraint here: a query
//! talks to a machine nobody vetted, running software nobody vetted, which can
//! answer with anything or nothing. Every read has a deadline and a ceiling, so
//! the worst a hostile server achieves is one slow, capped exchange.
//!
//! Nothing in this module resolves a hostname — callers pass a `SocketAddr`
//! that already came from [`crate::net::addr::resolve_public`]. Re-resolving
//! here would reopen the rebinding hole that function exists to close.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

use crate::error::{AppError, AppResult};

/// Per-exchange ceiling. A server that has not answered in three seconds is
/// "unreachable" as far as a browsing user is concerned.
pub const MAX_TIMEOUT_MS: u64 = 3_000;

/// Response cap. Larger than any single-datagram query reply, far below
/// anything that could be used to make the app allocate.
pub const MAX_UDP_RESPONSE: usize = 16 * 1024;

/// Request cap. A query header, not a payload.
pub const MAX_UDP_REQUEST: usize = 1_024;

/// TCP response cap. Minecraft's status JSON is the largest thing we read and
/// runs to a few tens of KB with a base64 favicon.
pub const MAX_TCP_RESPONSE: usize = 256 * 1024;

pub fn clamp_timeout(ms: u64) -> Duration {
    Duration::from_millis(ms.clamp(100, MAX_TIMEOUT_MS))
}

/// A bound UDP socket with a fixed peer, for protocols needing more than one
/// round trip from the *same* source port.
///
/// That requirement is not theoretical: Valve's A2S challenge is issued against
/// the querying address **and port**, so sending the challenged retry from a
/// fresh ephemeral port makes several server builds ignore it — presenting as
/// "this server never answers".
pub struct UdpSession {
    socket: UdpSocket,
}

impl UdpSession {
    pub async fn connect(addr: SocketAddr) -> AppResult<Self> {
        let bind: SocketAddr = if addr.is_ipv6() {
            "[::]:0".parse().expect("static addr")
        } else {
            "0.0.0.0:0".parse().expect("static addr")
        };

        let socket = UdpSocket::bind(bind).await?;

        /*
         * `connect` on a UDP socket sets the peer, which makes the kernel drop
         * datagrams from anyone else. Without it a third party who can guess
         * the ephemeral port can answer on the real server's behalf — trivially,
         * since the query is unauthenticated and the port is observable to
         * anyone on the path.
         */
        socket.connect(addr).await?;

        Ok(Self { socket })
    }

    /// One request, one reply.
    pub async fn exchange(&self, payload: &[u8], timeout: Duration) -> AppResult<(Vec<u8>, u32)> {
        if payload.is_empty() || payload.len() > MAX_UDP_REQUEST {
            return Err(AppError::invalid("Query payload is empty or too large."));
        }

        let started = Instant::now();

        self.socket.send(payload).await?;

        let mut buf = vec![0u8; MAX_UDP_RESPONSE];

        let read = tokio::time::timeout(timeout, self.socket.recv(&mut buf))
            .await
            .map_err(|_| AppError::Network("The server did not answer.".into()))??;

        buf.truncate(read);

        Ok((buf, elapsed_ms(started)))
    }

    /// Read another datagram the server sent unprompted — the continuation of a
    /// split reply. Bounded by the caller's deadline.
    pub async fn recv_more(&self, timeout: Duration) -> AppResult<Vec<u8>> {
        let mut buf = vec![0u8; MAX_UDP_RESPONSE];

        let read = tokio::time::timeout(timeout, self.socket.recv(&mut buf))
            .await
            .map_err(|_| AppError::Network("The server stopped sending.".into()))??;

        buf.truncate(read);

        Ok(buf)
    }
}

/// One UDP request/response exchange on a throwaway socket.
///
/// Deliberately a single round trip. A "keep reading until the server stops"
/// primitive is an unbounded read driven by a remote host; protocols needing
/// several datagrams use [`UdpSession`] and bound the count themselves.
pub async fn udp_exchange(
    addr: SocketAddr,
    payload: &[u8],
    timeout: Duration,
) -> AppResult<(Vec<u8>, u32)> {
    UdpSession::connect(addr)
        .await?
        .exchange(payload, timeout)
        .await
}

/// Several datagrams from one request — Source's split `A2S_PLAYER` replies and
/// GameSpy 3's multi-packet responses.
///
/// Bounded by BOTH a packet count and the overall deadline, because either
/// alone is escapable: a count-only bound lets a server trickle `max_packets`
/// replies at one per timeout, and a deadline-only bound lets it flood.
pub async fn udp_exchange_multi(
    addr: SocketAddr,
    payload: &[u8],
    timeout: Duration,
    max_packets: usize,
) -> AppResult<(Vec<Vec<u8>>, u32)> {
    if payload.is_empty() || payload.len() > MAX_UDP_REQUEST {
        return Err(AppError::invalid("Query payload is empty or too large."));
    }

    let bind: SocketAddr = if addr.is_ipv6() {
        "[::]:0".parse().expect("static addr")
    } else {
        "0.0.0.0:0".parse().expect("static addr")
    };

    let socket = UdpSocket::bind(bind).await?;
    socket.connect(addr).await?;

    let started = Instant::now();

    socket.send(payload).await?;

    let mut packets: Vec<Vec<u8>> = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;

    while packets.len() < max_packets {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());

        if remaining.is_zero() {
            break;
        }

        let mut buf = vec![0u8; MAX_UDP_RESPONSE];

        match tokio::time::timeout(remaining, socket.recv(&mut buf)).await {
            Ok(Ok(read)) => {
                buf.truncate(read);
                packets.push(buf);
            }
            // A timeout after at least one packet is the normal way a
            // multi-packet reply ends — there is no terminator.
            Ok(Err(_)) | Err(_) => break,
        }
    }

    if packets.is_empty() {
        return Err(AppError::Network("The server did not answer.".into()));
    }

    Ok((packets, elapsed_ms(started)))
}

/// One TCP request/response exchange, reading until EOF or the cap.
pub async fn tcp_exchange(
    addr: SocketAddr,
    payload: &[u8],
    timeout: Duration,
) -> AppResult<(Vec<u8>, u32)> {
    let started = Instant::now();

    let mut stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| AppError::Network("The server did not answer.".into()))??;

    // Nagle would hold a small query header waiting for more data that is never
    // coming, adding ~40ms to every measurement.
    let _ = stream.set_nodelay(true);

    let connect_ms = elapsed_ms(started);

    tokio::time::timeout(timeout, stream.write_all(payload))
        .await
        .map_err(|_| AppError::Network("The server stopped responding.".into()))??;

    let mut out = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];

    loop {
        let read = match tokio::time::timeout(timeout, stream.read(&mut chunk)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => n,
            // A read timeout with data already in hand is how most of these
            // protocols end; the server simply stops rather than closing.
            Ok(Err(_)) | Err(_) => break,
        };

        out.extend_from_slice(&chunk[..read]);

        if out.len() >= MAX_TCP_RESPONSE {
            out.truncate(MAX_TCP_RESPONSE);
            break;
        }
    }

    if out.is_empty() {
        return Err(AppError::Network("The server did not answer.".into()));
    }

    Ok((out, connect_ms))
}

pub fn elapsed_ms(started: Instant) -> u32 {
    started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
}
