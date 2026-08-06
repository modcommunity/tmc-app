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

    LatencySeries {
        key: key.to_string(),
        last: samples.iter().rev().find_map(|s| s.rtt_ms),
        min,
        max,
        avg,
        jitter,
        reliability,
        samples,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
