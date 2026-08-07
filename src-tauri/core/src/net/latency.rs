//! Rolling latency history, per server, for the graphs.
//!
//! A single ping is close to meaningless — Wi-Fi jitter, a busy CPU and one
//! unlucky route all produce numbers that move by 50ms between samples. What a
//! player actually wants to know is whether a server is *consistently* close,
//! and that needs a series.
//!
//! Held in memory, not on disk. The series is only interesting while the
//! browser is open, it is entirely reconstructible by re-measuring, and
//! persisting it would mean writing a file on every scroll tick.
//!
//! Bounded on both axes: [`MAX_SAMPLES`] per server and [`MAX_SERVERS`]
//! tracked. Without the second bound a long scroll through a few thousand
//! servers is an unbounded map keyed by attacker-influenced strings.

use std::collections::{BTreeMap, VecDeque};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// Samples kept per server. At the browser's ~10s cadence this is about ten
/// minutes of history, which is the useful window for "is this stable?".
pub const MAX_SAMPLES: usize = 60;

/// Servers tracked at once. Eviction is oldest-touched-first.
pub const MAX_SERVERS: usize = 512;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencySample {
    /// Milliseconds since the Unix epoch, stamped by the caller.
    pub at: i64,
    /// `None` records a failed probe — a timeout is data, and dropping it makes
    /// an intermittently-dead server look perfectly stable.
    pub rtt_ms: Option<u32>,
}

/// The series plus the numbers the UI actually renders.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LatencySeries {
    pub key: String,
    pub samples: Vec<LatencySample>,

    /// Most recent successful sample.
    pub last: Option<u32>,
    pub min: Option<u32>,
    pub max: Option<u32>,
    /// Mean over successful samples only.
    pub avg: Option<u32>,
    /// Mean absolute deviation from `avg`. Cheaper than a standard deviation
    /// and reads the same way on a sparkline: small means stable.
    pub jitter: Option<u32>,
    /// Successful probes as a percentage, 0–100.
    pub reliability: u8,

    /// Middle successful sample. Reported alongside `avg` because a bimodal
    /// series has a mean that describes neither mode — which is precisely the
    /// series {@link CacheSignal} is about.
    pub median: Option<u32>,

    /// Evidence that something is answering this query from a cache. `None`
    /// until there are enough samples to say anything.
    pub cache: Option<CacheSignal>,
}

/// "A cache is answering some of these probes."
///
/// WHY THIS IS DETECTABLE AT ALL
/// -----------------------------
/// A cache in front of a game's query port is not fast on every probe. It is
/// fast on every probe that HITS, and a TTL shorter than the probe interval
/// guarantees the rest miss and pay the full round trip. So the series goes
/// BIMODAL — a tight cluster of very fast replies and a second cluster at the
/// real network latency — in a way that ordinary jitter does not.
///
/// THE MISTAKE THIS AVOIDS
/// -----------------------
/// The obvious test is "are these samples consistent?", and it is exactly
/// backwards: the slow samples ARE the cache's signature, so requiring their
/// absence vetoes the servers where the effect is strongest. (The website hit
/// this on its own info-vs-userlist detector; see `latency_cache.ts` there.)
/// So this asks the opposite question — *are there two populations?* — and both
/// modes have to be genuinely populated for it to fire.
///
/// WHAT IT IS NOT
/// --------------
/// Not an accusation. A CDN, a proxy and a game that genuinely answers some
/// requests from memory all produce this, and none of them is wrongdoing. It is
/// shown so a player reads "12ms" with the right amount of trust.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheSignal {
    /// Median of the slow cluster ÷ median of the fast one. The bigger this is,
    /// the less the fast number is telling you about the network.
    pub ratio: f32,
    /// Share of probes that landed in the fast cluster, 0–1. A cache with a
    /// long TTL is near 1; one expiring between probes is near 0.5.
    pub fast_share: f32,
    /// The fast cluster's median, in ms — what the server appears to answer in.
    pub fast_ms: u32,
    /// The slow cluster's median — the round trip when the cache misses, which
    /// is the number a player's connection will actually experience.
    pub slow_ms: u32,
}

/// Samples needed before a split is worth reading. Below this a single unlucky
/// probe is half the "slow cluster".
const CACHE_MIN_SAMPLES: usize = 8;

/// Each cluster has to hold at least this share of the samples. This is the
/// bound that separates a cache from one outlier: a lone 400ms spike among
/// nineteen 12ms replies is jitter, and it lands at 5%.
const CACHE_MIN_CLUSTER_SHARE: f32 = 0.15;

/// How much slower the slow cluster has to be. Chosen well above the ~2x a
/// merely larger reply costs — the same band the website's detector treats as
/// "a bigger response, not a cache".
const CACHE_MIN_RATIO: f32 = 4.0;

fn median_of(sorted: &[u32]) -> Option<u32> {
    if sorted.is_empty() {
        return None;
    }

    Some(sorted[sorted.len() / 2])
}

/// Split a sorted series at its widest internal gap and judge the two halves.
///
/// The widest gap rather than the mean or the median: a bimodal series has one
/// large step between its clusters and small steps inside them, which is the
/// thing being looked for. Splitting at the mean would put a boundary through
/// the middle of a unimodal series and then measure the halves against each
/// other, which always finds *something*.
fn detect_cache(successful: &[u32]) -> Option<CacheSignal> {
    if successful.len() < CACHE_MIN_SAMPLES {
        return None;
    }

    let mut sorted = successful.to_vec();
    sorted.sort_unstable();

    let mut best_at = 0usize;
    let mut best_gap = 0u32;

    for i in 1..sorted.len() {
        let gap = sorted[i].saturating_sub(sorted[i - 1]);

        if gap > best_gap {
            best_gap = gap;
            best_at = i;
        }
    }

    if best_at == 0 {
        return None;
    }

    let (fast, slow) = sorted.split_at(best_at);

    let total = sorted.len() as f32;
    let fast_share = fast.len() as f32 / total;
    let slow_share = slow.len() as f32 / total;

    if fast_share < CACHE_MIN_CLUSTER_SHARE || slow_share < CACHE_MIN_CLUSTER_SHARE {
        return None;
    }

    let fast_ms = median_of(fast)?;
    let slow_ms = median_of(slow)?;

    // A fast cluster at literally 0ms would divide by zero; clamp to 1, which
    // also stops a sub-millisecond LAN reply producing an absurd ratio.
    let ratio = slow_ms as f32 / fast_ms.max(1) as f32;

    if ratio < CACHE_MIN_RATIO {
        return None;
    }

    Some(CacheSignal {
        ratio,
        fast_share,
        fast_ms,
        slow_ms,
    })
}

#[derive(Default)]
struct Entry {
    samples: VecDeque<LatencySample>,
    /// Monotonic counter, for eviction. Not a clock — the caller's timestamps
    /// come from the webview and a device with a wrong clock must not be able
    /// to make its own entries un-evictable.
    touched: u64,
}

pub struct LatencyStore {
    inner: RwLock<Inner>,
}

#[derive(Default)]
struct Inner {
    entries: BTreeMap<String, Entry>,
    tick: u64,
}

impl Default for LatencyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner::default()),
        }
    }

    /// Canonical key for a server. Lower-cased so `Host:1` and `HOST:1` are one
    /// series rather than two half-length ones.
    pub fn key(host: &str, port: u16) -> String {
        format!("{}:{port}", host.trim().to_ascii_lowercase())
    }

    pub fn record(&self, key: &str, at: i64, rtt_ms: Option<u32>) {
        let Ok(mut inner) = self.inner.write() else {
            // A poisoned lock means another thread panicked mid-write. Latency
            // history is decoration; losing a sample is the right trade
            // against propagating the panic.
            return;
        };

        inner.tick = inner.tick.wrapping_add(1);
        let tick = inner.tick;

        if !inner.entries.contains_key(key) && inner.entries.len() >= MAX_SERVERS {
            /*
             * Evict the least-recently-touched entry. Doing this BEFORE the
             * insert is what keeps the map at its bound rather than one over.
             */
            if let Some(oldest) = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(k, _)| k.clone())
            {
                inner.entries.remove(&oldest);
            }
        }

        let entry = inner.entries.entry(key.to_string()).or_default();

        entry.touched = tick;
        entry.samples.push_back(LatencySample { at, rtt_ms });

        while entry.samples.len() > MAX_SAMPLES {
            entry.samples.pop_front();
        }
    }

    pub fn series(&self, key: &str) -> Option<LatencySeries> {
        let inner = self.inner.read().ok()?;
        let entry = inner.entries.get(key)?;

        Some(summarise(key, entry.samples.iter().copied().collect()))
    }

    /// Several series at once — one IPC round trip for a whole grid.
    pub fn many(&self, keys: &[String]) -> Vec<LatencySeries> {
        let Ok(inner) = self.inner.read() else {
            return Vec::new();
        };

        keys.iter()
            .filter_map(|key| {
                let entry = inner.entries.get(key)?;

                Some(summarise(key, entry.samples.iter().copied().collect()))
            })
            .collect()
    }

    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.write() {
            inner.entries.clear();
        }
    }

    pub fn tracked(&self) -> usize {
        self.inner.read().map(|i| i.entries.len()).unwrap_or(0)
    }
}

fn summarise(key: &str, samples: Vec<LatencySample>) -> LatencySeries {
    let successful: Vec<u32> = samples.iter().filter_map(|s| s.rtt_ms).collect();

    let reliability = if samples.is_empty() {
        0
    } else {
        // Rounded rather than truncated: 59/60 successes should read as 98%,
        // not 98% only because the floor happened to agree.
        (((successful.len() as f64) / (samples.len() as f64)) * 100.0).round() as u8
    };

    let (min, max, avg, jitter) = if successful.is_empty() {
        (None, None, None, None)
    } else {
        let sum: u64 = successful.iter().map(|v| u64::from(*v)).sum();
        let avg = (sum / successful.len() as u64) as u32;

        let deviation: u64 = successful
            .iter()
            .map(|v| u64::from(v.abs_diff(avg)))
            .sum::<u64>()
            / successful.len() as u64;

        (
            successful.iter().copied().min(),
            successful.iter().copied().max(),
            Some(avg),
            Some(deviation as u32),
        )
    };

    let mut sorted = successful.clone();
    sorted.sort_unstable();

    LatencySeries {
        key: key.to_string(),
        last: samples.iter().rev().find_map(|s| s.rtt_ms),
        min,
        max,
        avg,
        jitter,
        reliability,
        median: median_of(&sorted),
        cache: detect_cache(&successful),
        samples,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the detector, stated as a test.
    ///
    /// A cache in front of a query port is fast on every HIT and pays the full
    /// round trip on every miss. The series below is a real shape: fourteen
    /// ~12ms replies and six ~540ms ones. A consistency test would call that
    /// unstable and say nothing; this has to call it a cache.
    #[test]
    fn a_bimodal_series_is_read_as_a_cache_not_as_jitter() {
        let mut samples: Vec<u32> = vec![11, 12, 12, 13, 12, 11, 14, 12, 12, 13, 11, 12, 13, 12];
        samples.extend([540, 552, 536, 549, 541, 558]);

        let signal = detect_cache(&samples).expect("a cache should be detected");

        assert!(signal.ratio > 30.0, "ratio was {}", signal.ratio);
        assert!(signal.fast_ms < 20);
        assert!(signal.slow_ms > 500);
        assert!(
            (signal.fast_share - 0.7).abs() < 0.05,
            "share was {}",
            signal.fast_share
        );
    }

    /// The inverse, and the reason the cluster-share bound exists: one unlucky
    /// probe among nineteen good ones is jitter, and must not fire.
    #[test]
    fn a_single_outlier_is_not_a_cache() {
        let mut samples: Vec<u32> = vec![12; 19];
        samples.push(600);

        assert!(detect_cache(&samples).is_none());
    }

    /// A merely LARGER reply costs about twice as long, not five times. That
    /// band is the one the website's own detector treats as "a bigger response,
    /// not a cache", and the ratio floor here has to agree.
    #[test]
    fn a_two_times_gap_is_a_bigger_reply_not_a_cache() {
        let mut samples: Vec<u32> = vec![40, 42, 41, 43, 40, 42, 41, 40, 42, 41];
        samples.extend([80, 84, 82, 81, 83, 80, 82, 84, 81, 80]);

        assert!(detect_cache(&samples).is_none());
    }

    /// An ordinary noisy connection — a spread, but one population.
    #[test]
    fn ordinary_jitter_does_not_fire() {
        let samples: Vec<u32> = vec![40, 55, 47, 62, 51, 44, 58, 49, 66, 53, 45, 60];

        assert!(detect_cache(&samples).is_none());
    }

    /// Below the sample floor nothing is claimed, however suggestive the shape.
    #[test]
    fn a_short_series_says_nothing() {
        assert!(detect_cache(&[10, 11, 500, 520]).is_none());
    }

    /// A sub-millisecond LAN reply must not divide by zero or produce an
    /// absurd ratio off a rounding artefact.
    #[test]
    fn a_zero_millisecond_cluster_does_not_divide_by_zero() {
        let mut samples: Vec<u32> = vec![0; 10];
        samples.extend([120; 10]);

        let signal = detect_cache(&samples).expect("still bimodal");

        assert!(signal.ratio.is_finite());
        assert_eq!(signal.fast_ms, 0);
    }

    #[test]
    fn the_summary_carries_the_signal_and_a_median() {
        let store = LatencyStore::new();
        let key = LatencyStore::key("cached.test", 1);

        for (i, rtt) in [10, 11, 10, 12, 11, 10, 11, 12, 500, 510, 505, 520]
            .into_iter()
            .enumerate()
        {
            store.record(&key, i as i64, Some(rtt));
        }

        let series = store.series(&key).expect("series");

        assert!(series.median.is_some());
        assert!(series.cache.is_some(), "expected a cache signal");
    }

    #[test]
    fn keys_are_case_insensitive() {
        assert_eq!(
            LatencyStore::key("Example.COM", 27015),
            LatencyStore::key("example.com", 27015)
        );
        assert_eq!(LatencyStore::key("  host ", 1), "host:1");
    }

    #[test]
    fn summarises_a_series() {
        let store = LatencyStore::new();
        let key = LatencyStore::key("a.test", 1);

        for (i, rtt) in [Some(10), Some(20), Some(30), None].into_iter().enumerate() {
            store.record(&key, i as i64, rtt);
        }

        let series = store.series(&key).expect("series");

        assert_eq!(series.samples.len(), 4);
        assert_eq!(series.min, Some(10));
        assert_eq!(series.max, Some(30));
        assert_eq!(series.avg, Some(20));
        assert_eq!(series.last, Some(30));
        assert_eq!(series.reliability, 75);
        // Mean absolute deviation of 10/20/30 around 20 is 6 (20/3 floored).
        assert_eq!(series.jitter, Some(6));
    }

    #[test]
    fn a_failed_probe_is_recorded_not_dropped() {
        let store = LatencyStore::new();
        let key = LatencyStore::key("b.test", 1);

        store.record(&key, 0, None);
        store.record(&key, 1, None);

        let series = store.series(&key).expect("series");

        assert_eq!(series.reliability, 0);
        assert_eq!(series.last, None);
        assert_eq!(series.avg, None);
        assert_eq!(series.samples.len(), 2);
    }

    #[test]
    fn samples_are_bounded_per_server() {
        let store = LatencyStore::new();
        let key = LatencyStore::key("c.test", 1);

        for i in 0..(MAX_SAMPLES * 3) {
            store.record(&key, i as i64, Some(i as u32));
        }

        let series = store.series(&key).expect("series");

        assert_eq!(series.samples.len(), MAX_SAMPLES);
        // The window keeps the NEWEST samples.
        assert_eq!(
            series.samples.last().map(|s| s.rtt_ms),
            Some(Some((MAX_SAMPLES * 3 - 1) as u32))
        );
    }

    #[test]
    fn the_server_count_is_bounded_and_evicts_the_oldest() {
        let store = LatencyStore::new();

        for i in 0..(MAX_SERVERS + 50) {
            store.record(&LatencyStore::key(&format!("h{i}.test"), 1), 0, Some(1));
        }

        assert!(store.tracked() <= MAX_SERVERS);

        // The first host in should be gone; the last should still be there.
        assert!(store.series(&LatencyStore::key("h0.test", 1)).is_none());
        assert!(store
            .series(&LatencyStore::key(
                &format!("h{}.test", MAX_SERVERS + 49),
                1
            ))
            .is_some());
    }

    #[test]
    fn touching_an_entry_protects_it_from_eviction() {
        let store = LatencyStore::new();
        let kept = LatencyStore::key("kept.test", 1);

        store.record(&kept, 0, Some(1));

        for i in 0..(MAX_SERVERS - 1) {
            store.record(&LatencyStore::key(&format!("h{i}.test"), 1), 0, Some(1));
        }

        // Re-touch, then push the map over its bound.
        store.record(&kept, 1, Some(2));

        for i in 0..20 {
            store.record(&LatencyStore::key(&format!("late{i}.test"), 1), 0, Some(1));
        }

        assert!(store.series(&kept).is_some());
    }

    #[test]
    fn many_returns_only_the_series_that_exist() {
        let store = LatencyStore::new();
        let a = LatencyStore::key("a.test", 1);

        store.record(&a, 0, Some(5));

        let out = store.many(&[a.clone(), LatencyStore::key("missing.test", 1)]);

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, a);
    }

    #[test]
    fn an_empty_series_summarises_without_dividing_by_zero() {
        let series = summarise("x", vec![]);

        assert_eq!(series.reliability, 0);
        assert_eq!(series.avg, None);
        assert_eq!(series.jitter, None);
    }
}
