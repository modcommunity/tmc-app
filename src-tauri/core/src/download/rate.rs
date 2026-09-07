//! Bandwidth limiting, as a token bucket.
//!
//! WHY A LIMITER AT ALL
//! --------------------
//! A mod manager downloading a 12 GB modpack at line rate makes the machine's
//! network unusable for everything else on it — including, on a home
//! connection, everybody else in the house. Every mature downloader has this
//! setting and it is one of the most-used ones.
//!
//! WHY A TOKEN BUCKET
//! ------------------
//! It is the only shape that gets both halves right. A limiter that sleeps for
//! a fixed slice per chunk is wrong whenever chunks are not a fixed size (they
//! never are), and one that measures average rate and corrects is either slow
//! to react or oscillates. A bucket refilled continuously and drained by actual
//! bytes is exact over any window and needs no history.
//!
//! **The burst is one second's worth**, floored at 32 KiB. Smaller and a fast
//! link spends its time asleep between single packets; larger and a limit set
//! to 1 MB/s lets 4 MB through the moment a download starts, which is exactly
//! when somebody watching a video notices.
//!
//! COMPOSING TWO LIMITS
//! --------------------
//! A download passes through the global limiter and its own, in that order, and
//! must satisfy both — so `min(global, per-download)` is the effective rate
//! without either needing to know about the other. [`Limiter::unlimited`] is
//! the "no setting" case and costs a single atomic read.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

/// The smallest burst worth allowing, in bytes.
const MIN_BURST: f64 = 32.0 * 1024.0;

/// Never sleep for less than this. A thousand 200-microsecond sleeps cost more
/// in wakeups than they save in bandwidth.
const MIN_SLEEP: Duration = Duration::from_millis(2);

/// Never sleep for longer than this in one go, so a rate change or a cancel is
/// noticed promptly.
const MAX_SLEEP: Duration = Duration::from_millis(250);

struct Bucket {
    /// Bytes per second. `None` is unlimited.
    rate: Option<u64>,
    tokens: f64,
    last: Instant,
}

/// A shared, adjustable rate limit.
#[derive(Clone)]
pub struct Limiter {
    bucket: Arc<Mutex<Bucket>>,
}

impl Limiter {
    pub fn new(rate_bps: Option<u64>) -> Self {
        Self {
            bucket: Arc::new(Mutex::new(Bucket {
                rate: rate_bps.filter(|r| *r > 0),
                tokens: 0.0,
                last: Instant::now(),
            })),
        }
    }

    pub fn unlimited() -> Self {
        Self::new(None)
    }

    /// Change the rate while downloads are running.
    ///
    /// The bucket is NOT reset. Draining it on a change would let somebody
    /// lower the limit and have the next second run at the old one, and
    /// clearing it would stall an in-flight read for a full second on a raise.
    pub async fn set_rate(&self, rate_bps: Option<u64>) {
        let mut bucket = self.bucket.lock().await;

        bucket.rate = rate_bps.filter(|r| *r > 0);

        // Clamp what is already banked to the new burst, so lowering the limit
        // takes effect immediately rather than after the old burst is spent.
        if let Some(rate) = bucket.rate {
            bucket.tokens = bucket.tokens.min(burst_for(rate));
        }
    }

    pub async fn rate(&self) -> Option<u64> {
        self.bucket.lock().await.rate
    }

    /// Wait until `bytes` may be sent, then account for them.
    ///
    /// Returns immediately when unlimited. A request LARGER than the burst is
    /// still served — it drains the bucket negative and the next call waits for
    /// it to recover — because refusing it would deadlock a downloader whose
    /// chunk size it does not control.
    pub async fn take(&self, bytes: usize) {
        loop {
            let wait = {
                let mut bucket = self.bucket.lock().await;

                let Some(rate) = bucket.rate else {
                    return;
                };

                let now = Instant::now();
                let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();

                bucket.last = now;
                bucket.tokens = (bucket.tokens + elapsed * rate as f64).min(burst_for(rate));

                let need = bytes as f64;

                /*
                 * A request larger than the bucket can ever hold must still be
                 * served, or a downloader whose chunk size it does not control
                 * hangs forever. So the wait is for the bucket to be as full as
                 * it CAN be — `min(need, burst)` — and then the full cost is
                 * paid, taking the balance negative.
                 *
                 * The debt is repaid by the same refill as everything else, so
                 * the average rate over any window is still exactly `rate`; the
                 * only difference is that one oversized read is followed by a
                 * proportionally longer pause instead of being refused.
                 */
                let bankable = need.min(burst_for(rate));

                if bucket.tokens >= bankable {
                    bucket.tokens -= need;

                    return;
                }

                let short = bankable - bucket.tokens;

                Duration::from_secs_f64(short / rate as f64)
            };

            tokio::time::sleep(wait.clamp(MIN_SLEEP, MAX_SLEEP)).await;
        }
    }
}

/// One second's worth, or `MIN_BURST` — see the module header.
fn burst_for(rate: u64) -> f64 {
    (rate as f64).max(MIN_BURST)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_unlimited_limiter_never_waits() {
        let limiter = Limiter::unlimited();

        let started = Instant::now();

        for _ in 0..1000 {
            limiter.take(1024 * 1024).await;
        }

        assert!(
            started.elapsed() < Duration::from_millis(200),
            "unlimited must not be a cost"
        );
    }

    #[tokio::test]
    async fn a_limited_bucket_paces_bytes_over_time() {
        // 100 KiB/s, so 200 KiB is about two seconds' worth. The burst is one
        // second, so the first ~100 KiB is free and the rest is paced.
        let limiter = Limiter::new(Some(100 * 1024));

        let started = Instant::now();

        for _ in 0..20 {
            limiter.take(10 * 1024).await;
        }

        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(700),
            "200 KiB at 100 KiB/s cannot finish in {elapsed:?}"
        );

        assert!(
            elapsed < Duration::from_secs(4),
            "and must not take dramatically longer either: {elapsed:?}"
        );
    }

    /// The case that deadlocks a naive implementation: a chunk bigger than the
    /// bucket can ever hold.
    #[tokio::test]
    async fn a_request_larger_than_the_burst_still_completes() {
        let limiter = Limiter::new(Some(8 * 1024));

        tokio::time::timeout(Duration::from_secs(10), limiter.take(64 * 1024))
            .await
            .expect("an oversized request must not deadlock");
    }

    #[tokio::test]
    async fn lowering_the_rate_takes_effect_without_spending_the_old_burst() {
        let limiter = Limiter::new(Some(10 * 1024 * 1024));

        // Bank a large burst.
        limiter.take(1).await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        limiter.set_rate(Some(16 * 1024)).await;

        let started = Instant::now();

        // Two bursts' worth at the NEW rate.
        limiter.take(32 * 1024).await;
        limiter.take(32 * 1024).await;

        assert!(
            started.elapsed() >= Duration::from_millis(500),
            "the old burst must not carry over into the new limit"
        );
    }

    #[tokio::test]
    async fn a_rate_of_zero_reads_as_unlimited_rather_than_as_a_stall() {
        // The UI's "no limit" is an empty field, which arrives as 0. Treating
        // that as "zero bytes per second" would hang every download forever.
        let limiter = Limiter::new(Some(0));

        assert_eq!(limiter.rate().await, None);

        tokio::time::timeout(Duration::from_millis(500), limiter.take(1024 * 1024))
            .await
            .expect("zero must mean unlimited");
    }

    /// Two limits compose by both being satisfied, which is what the manager
    /// relies on to avoid either knowing about the other.
    #[tokio::test]
    async fn passing_through_two_limiters_yields_the_lower_rate() {
        let global = Limiter::new(Some(1024 * 1024));
        let per_download = Limiter::new(Some(32 * 1024));

        let started = Instant::now();

        for _ in 0..8 {
            global.take(8 * 1024).await;
            per_download.take(8 * 1024).await;
        }

        // 64 KiB at 32 KiB/s with a 32 KiB burst is about one second.
        assert!(
            started.elapsed() >= Duration::from_millis(600),
            "the tighter limit has to win"
        );
    }
}
