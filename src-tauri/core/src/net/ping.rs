//! Latency, measured from the device.
//!
//! ICMP is the obvious tool and the wrong one: a raw socket needs root on Linux
//! and macOS and is simply unavailable to a sandboxed iOS or Android app, so an
//! ICMP ping would work for exactly the developer testing it. A TCP handshake
//! to the port the user would actually connect on needs no privilege anywhere,
//! and measures the path that matters — including any middlebox between the
//! player and the server, which ICMP frequently routes around.
//!
//! The number is therefore *connect* RTT, not echo RTT. It reads a little
//! higher than a `ping` figure and is consistently comparable between servers,
//! which is what a browser column is for.
//!
//! Where a game speaks a protocol we implement, [`crate::net::query`] is
//! preferred: it produces a latency figure from the same round trip that
//! fetches the player count, so one exchange does both jobs.

use std::time::Instant;

use serde::Serialize;
use tokio::net::TcpStream;

use crate::error::AppResult;
use crate::net::addr::resolve_public;
use crate::net::transport::{clamp_timeout, elapsed_ms};

/// Attempts per probe. Three is enough to drop one unlucky sample without
/// making a 50-row grid open 150 sockets.
const MAX_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub host: String,
    pub port: u16,
    /// Best of the attempts, milliseconds. `None` when every attempt failed.
    pub rtt_ms: Option<u32>,
    /// How many attempts answered.
    pub received: u8,
    pub attempts: u8,
}

pub async fn tcp_ping(
    host: &str,
    port: u16,
    timeout_ms: u64,
    attempts: u8,
) -> AppResult<PingResult> {
    let addr = resolve_public(host, port).await?;

    let timeout = clamp_timeout(timeout_ms);
    let attempts = attempts.clamp(1, MAX_ATTEMPTS);

    let mut best: Option<u32> = None;
    let mut received = 0u8;

    for _ in 0..attempts {
        let started = Instant::now();

        // A refused connection still proves the round trip happened, but it
        // is not a server we can report a latency for, so only a completed
        // handshake counts.
        if let Ok(Ok(stream)) = tokio::time::timeout(timeout, TcpStream::connect(addr)).await {
            let rtt = elapsed_ms(started);

            received += 1;
            best = Some(best.map_or(rtt, |b: u32| b.min(rtt)));

            // Closed immediately: an open socket shows up as a phantom player
            // slot on several game servers.
            drop(stream);
        }
    }

    Ok(PingResult {
        host: host.to_string(),
        port,
        rtt_ms: best,
        received,
        attempts,
    })
}
