//! **The download manager** — a queue, not a `fetch`.
//!
//! Every file this app pulls down goes through here: a mod's release archive, a
//! modpack's forty dependencies, whatever a plugin's `download` step names. The
//! reason it is a subsystem rather than a function is that a modpack is not one
//! download, it is four hundred, over a home connection, on a laptop that will
//! be closed halfway through.
//!
//! ```text
//!   enqueue ──▶ queued ──▶ running ──┬──▶ done
//!                  ▲         │       ├──▶ failed ──▶ (retry) ──▶ queued
//!                  └─────────┴───────┴──▶ paused ──▶ (resume) ──▶ queued
//! ```
//!
//! WHAT IT GUARANTEES
//! ------------------
//!   * **Bytes already downloaded are never downloaded twice.** Progress lives
//!     in a `.part` file beside the target, and a resume sends
//!     `Range: bytes=<n>-`. A server that ignores the range is detected (it
//!     answers `200`, not `206`) and the file restarts rather than being
//!     silently concatenated — the failure that produces a corrupt archive
//!     whose checksum error nobody can explain.
//!   * **The partial file is never mistaken for the real one.** The target name
//!     only ever appears after the whole transfer and its checksum have passed.
//!   * **The user's connection stays usable.** A global limit and a per-download
//!     limit, both live-adjustable — see [`rate`].
//!   * **Nothing is unbounded.** Concurrency, retries, queue length, the speed
//!     history and the size of a single file all have ceilings.
//!
//! WHAT IT IS NOT
//! --------------
//! It does not decide *what* to download, and it does not resolve paths. The
//! caller hands it an absolute destination it has already put through the
//! plugin jail, and a URL it has already decided is allowed. Two things this
//! module does check, because they are cheap and the cost of missing them is
//! high: the URL must be `https`, and its host must not resolve to a private
//! address (the same guard the server browser uses, for the same SSRF reason).

pub mod rate;
pub mod store;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, Mutex};

use crate::error::{AppError, AppResult};
use crate::logging::{now_rfc3339, Audit};
use crate::{audit, net};

pub use rate::Limiter;

/// Cap on queued-or-running downloads. A modpack is a few hundred; ten thousand
/// is a bug somewhere upstream and should be refused where it can be explained.
pub const MAX_QUEUE: usize = 10_000;

/// Cap on simultaneous transfers. More than this is slower, not faster — the
/// bottleneck moves to the link and every file finishes late instead of some
/// finishing early.
pub const MAX_CONCURRENT: usize = 8;

/// Default simultaneous transfers.
pub const DEFAULT_CONCURRENT: usize = 3;

/// Hard ceiling on one file.
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024 * 1024;

/// Automatic retries before a download is left failed for the user to decide.
const MAX_ATTEMPTS: u32 = 3;

/// Speed samples kept per download — two minutes at the half-second tick. A
/// COUNT, not a window, exactly as the latency graph's history is.
const MAX_SAMPLES: usize = 240;

/// How often progress is sampled and published.
const TICK: Duration = Duration::from_millis(500);

/// Where a download is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Queued,
    Running,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "paused" => Some(Self::Paused),
            "done" => Some(Self::Done),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// Is this download still going to do something?
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }

    pub fn is_finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// What to download.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
    /// Caller-supplied and stable, so re-enqueuing the same thing is idempotent
    /// rather than producing a second row. A sandbox staging a mod uses
    /// `sb<id>:<mod key>`.
    pub id: String,
    pub url: String,
    /// Absolute, and already resolved through the jail by the caller.
    pub dest: PathBuf,
    /// What the user sees.
    pub label: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size_hint: Option<u64>,
    /// Higher runs first. Ties break by enqueue order.
    #[serde(default)]
    pub priority: i32,
    /// This download's own ceiling, on top of the global one.
    #[serde(default)]
    pub limit_bps: Option<u64>,
    /// Echoed back on every snapshot, untouched. The sandbox id and mod key
    /// travel here so a completion can be routed without this module knowing
    /// what a sandbox is.
    #[serde(default)]
    pub meta: BTreeMap<String, String>,
}

/// One download, as anything outside this module sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadState {
    pub id: String,
    pub label: String,
    pub url: String,
    pub dest: String,
    pub status: Status,
    /// `None` until the server says, and it may never say.
    pub total: Option<u64>,
    pub done: u64,
    /// Bytes per second over the last tick.
    pub speed_bps: u64,
    /// Seconds, when both a total and a speed are known.
    pub eta_secs: Option<u64>,
    pub priority: i32,
    pub limit_bps: Option<u64>,
    pub attempts: u32,
    pub error: Option<String>,
    /// Expected digest, when the API gave us one. Verified after the last byte
    /// and before the file is given its real name.
    pub sha256: Option<String>,
    /// Recent speeds, oldest first — the graph reads this.
    pub samples: Vec<u64>,
    pub queued_at: String,
    pub updated_at: String,
    pub meta: BTreeMap<String, String>,
}

impl DownloadState {
    pub fn percent(&self) -> Option<f64> {
        let total = self.total?;

        if total == 0 {
            return None;
        }

        Some((self.done as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
    }
}

/// What the app subscribes to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum DownloadEvent {
    /// A download changed. Carries the whole row rather than a delta: the
    /// webview redraws one card from it, and a delta would need the receiver to
    /// hold state that can drift.
    Progress { download: Box<DownloadState> },
    /// Every active download finished, one way or another.
    Idle,
}

struct Control {
    cancel: AtomicBool,
    pause: AtomicBool,
    limiter: Limiter,
}

struct Entry {
    state: DownloadState,
    control: Arc<Control>,
    /// Monotonic, so equal priorities run in the order they arrived.
    seq: u64,
}

struct Registry {
    items: BTreeMap<String, Entry>,
    running: usize,
    max_concurrent: usize,
    seq: u64,
}

struct Inner {
    registry: Mutex<Registry>,
    global: Limiter,
    http: reqwest::Client,
    events: broadcast::Sender<DownloadEvent>,
    audit: Arc<Audit>,
}

/// The queue.
#[derive(Clone)]
pub struct DownloadManager {
    inner: Arc<Inner>,
}

/// An HTTP client for downloading FILES, distinct from the API's.
///
/// The API client attaches a bearer token to everything it sends, and a mod
/// archive comes from a CDN that has no business seeing one — a redirect to a
/// third-party mirror would hand somebody's access token to a host nobody here
/// controls. So the queue gets a plain client with no credentials attached to
/// anything, and the app crate does not need a `reqwest` dependency of its own
/// to build one.
pub fn default_client(version: &str) -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("TMC/{version}"))
        // Enough for a slow mirror to answer; the transfer itself is bounded by
        // the stream, not by this.
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::internal(format!("http client: {e}")))
}

impl DownloadManager {
    pub fn new(http: reqwest::Client, audit: Arc<Audit>) -> Self {
        let (events, _) = broadcast::channel(512);

        Self {
            inner: Arc::new(Inner {
                registry: Mutex::new(Registry {
                    items: BTreeMap::new(),
                    running: 0,
                    max_concurrent: DEFAULT_CONCURRENT,
                    seq: 0,
                }),
                global: Limiter::unlimited(),
                http,
                events,
                audit,
            }),
        }
    }

    /// Progress events. Every subscriber gets every event; a slow one loses the
    /// oldest, which for progress is exactly the right thing to drop.
    pub fn subscribe(&self) -> broadcast::Receiver<DownloadEvent> {
        self.inner.events.subscribe()
    }

    /// The global bandwidth ceiling. `None` (or zero) is unlimited.
    pub async fn set_global_limit(&self, bps: Option<u64>) {
        self.inner.global.set_rate(bps).await;
    }

    pub async fn global_limit(&self) -> Option<u64> {
        self.inner.global.rate().await
    }

    /// How many transfers may run at once, clamped to [`MAX_CONCURRENT`].
    pub async fn set_concurrency(&self, n: usize) {
        {
            let mut registry = self.inner.registry.lock().await;

            registry.max_concurrent = n.clamp(1, MAX_CONCURRENT);
        }

        self.pump().await;
    }

    /// Add one, or return the existing row if this id is already known.
    ///
    /// Idempotent by id. A sandbox that stages the same mod twice while the
    /// first attempt is still in flight must not produce two writers for one
    /// file, and making the caller check first is a race rather than a rule.
    pub async fn enqueue(&self, request: DownloadRequest) -> AppResult<DownloadState> {
        validate(&request)?;

        let state = {
            let mut registry = self.inner.registry.lock().await;

            if let Some(existing) = registry.items.get(&request.id) {
                // A finished row for the same id is replaced — "download it
                // again" is a real request. An active one is left alone.
                if existing.state.status.is_active() || existing.state.status == Status::Paused {
                    return Ok(existing.state.clone());
                }
            }

            if registry
                .items
                .values()
                .filter(|e| e.state.status.is_active())
                .count()
                >= MAX_QUEUE
            {
                return Err(AppError::invalid(
                    "The download queue is full. Let some finish first.",
                ));
            }

            registry.seq += 1;
            let seq = registry.seq;

            let now = now_rfc3339();

            let state = DownloadState {
                id: request.id.clone(),
                label: request.label.clone(),
                url: request.url.clone(),
                dest: request.dest.to_string_lossy().into_owned(),
                status: Status::Queued,
                total: request.size_hint,
                done: 0,
                speed_bps: 0,
                eta_secs: None,
                priority: request.priority,
                limit_bps: request.limit_bps,
                attempts: 0,
                error: None,
                sha256: request.sha256.clone(),
                samples: Vec::new(),
                queued_at: now.clone(),
                updated_at: now,
                meta: request.meta.clone(),
            };

            registry.items.insert(
                request.id.clone(),
                Entry {
                    state: state.clone(),
                    control: Arc::new(Control {
                        cancel: AtomicBool::new(false),
                        pause: AtomicBool::new(false),
                        limiter: Limiter::new(request.limit_bps),
                    }),
                    seq,
                },
            );

            state
        };

        self.publish(&state);
        self.pump().await;

        Ok(state)
    }

    /// Queue one download and wait for it to finish.
    ///
    /// What the plugin executor's `download` step calls, so a mod install shows
    /// up in the same queue — with the same progress bar, the same bandwidth
    /// limit and the same pause button — as anything else. A step that streamed
    /// its own bytes would be invisible to all three.
    ///
    /// A PAUSE resolves as an error rather than blocking. The install that is
    /// waiting cannot proceed, and leaving its task parked on a download the
    /// user may not resume for a week is worse than telling it so.
    pub async fn run_to_completion(&self, request: DownloadRequest) -> AppResult<()> {
        let id = request.id.clone();

        // Subscribed BEFORE enqueuing: a small file can finish before the first
        // `recv`, and a subscriber created afterwards would wait for an event
        // that has already been sent.
        let mut events = self.subscribe();

        let state = self.enqueue(request).await?;

        if let Some(outcome) = terminal(&state) {
            return outcome;
        }

        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), events.recv()).await;

            match event {
                Ok(Ok(DownloadEvent::Progress { download })) if download.id == id => {
                    if let Some(outcome) = terminal(&download) {
                        return outcome;
                    }
                }
                Ok(Ok(_)) => continue,
                /*
                 * Either the channel lagged (a busy queue outran this
                 * subscriber) or the timeout fired. Both are answered the same
                 * way: ask for the current state directly. Progress events are
                 * a convenience, not the source of truth.
                 */
                Ok(Err(broadcast::error::RecvError::Lagged(_))) | Err(_) => {
                    if let Some(state) = self.get(&id).await {
                        if let Some(outcome) = terminal(&state) {
                            return outcome;
                        }
                    } else {
                        return Err(AppError::internal(
                            "the download disappeared from the queue",
                        ));
                    }
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err(AppError::internal("the download queue stopped"))
                }
            }
        }
    }

    pub async fn list(&self) -> Vec<DownloadState> {
        let registry = self.inner.registry.lock().await;

        let mut out: Vec<DownloadState> =
            registry.items.values().map(|e| e.state.clone()).collect();

        // Active first, then by priority, then by arrival. The same order the
        // scheduler picks in, so the list reads as the queue it is.
        out.sort_by(|a, b| {
            rank(a.status)
                .cmp(&rank(b.status))
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| a.queued_at.cmp(&b.queued_at))
        });

        out
    }

    pub async fn get(&self, id: &str) -> Option<DownloadState> {
        self.inner
            .registry
            .lock()
            .await
            .items
            .get(id)
            .map(|e| e.state.clone())
    }

    /// Stop a running download but keep what it has.
    pub async fn pause(&self, id: &str) -> AppResult<()> {
        let state = {
            let mut registry = self.inner.registry.lock().await;

            let entry = registry
                .items
                .get_mut(id)
                .ok_or_else(|| AppError::invalid("No such download."))?;

            if entry.state.status.is_finished() {
                return Ok(());
            }

            entry.control.pause.store(true, Ordering::SeqCst);

            /*
             * A QUEUED download is paused here and now; a RUNNING one is paused
             * by its own task noticing the flag, so that it can close its file
             * cleanly. Setting the status here for a running one would show
             * "paused" while bytes were still arriving.
             */
            if entry.state.status == Status::Queued {
                entry.state.status = Status::Paused;
                entry.state.updated_at = now_rfc3339();
            }

            entry.state.clone()
        };

        self.publish(&state);

        Ok(())
    }

    /// Put a paused or failed download back in the queue.
    pub async fn resume(&self, id: &str) -> AppResult<()> {
        let state = {
            let mut registry = self.inner.registry.lock().await;

            let entry = registry
                .items
                .get_mut(id)
                .ok_or_else(|| AppError::invalid("No such download."))?;

            if entry.state.status == Status::Running {
                return Ok(());
            }

            entry.control.pause.store(false, Ordering::SeqCst);
            entry.control.cancel.store(false, Ordering::SeqCst);

            entry.state.status = Status::Queued;
            entry.state.error = None;
            // A manual retry starts the attempt count over. The automatic
            // retries are for a flaky minute; a person clicking retry has
            // usually changed something.
            entry.state.attempts = 0;
            entry.state.updated_at = now_rfc3339();

            entry.state.clone()
        };

        self.publish(&state);
        self.pump().await;

        Ok(())
    }

    /// Stop it and throw away the partial file.
    pub async fn cancel(&self, id: &str) -> AppResult<()> {
        let (state, part) = {
            let mut registry = self.inner.registry.lock().await;

            let entry = registry
                .items
                .get_mut(id)
                .ok_or_else(|| AppError::invalid("No such download."))?;

            entry.control.cancel.store(true, Ordering::SeqCst);

            let was_running = entry.state.status == Status::Running;

            if !was_running {
                entry.state.status = Status::Cancelled;
                entry.state.updated_at = now_rfc3339();
            }

            (
                entry.state.clone(),
                (!was_running).then(|| part_path(&PathBuf::from(&entry.state.dest))),
            )
        };

        // A running task removes its own `.part` when it sees the flag; doing
        // it here as well would delete a file another task still has open.
        if let Some(part) = part {
            let _ = std::fs::remove_file(part);
        }

        self.publish(&state);
        self.pump().await;

        Ok(())
    }

    /// Change one download's own bandwidth ceiling while it runs.
    pub async fn set_limit(&self, id: &str, bps: Option<u64>) -> AppResult<()> {
        let (control, state) = {
            let mut registry = self.inner.registry.lock().await;

            let entry = registry
                .items
                .get_mut(id)
                .ok_or_else(|| AppError::invalid("No such download."))?;

            entry.state.limit_bps = bps.filter(|b| *b > 0);
            entry.state.updated_at = now_rfc3339();

            (Arc::clone(&entry.control), entry.state.clone())
        };

        control.limiter.set_rate(bps).await;

        self.publish(&state);

        Ok(())
    }

    /// Move a download up or down the queue.
    pub async fn set_priority(&self, id: &str, priority: i32) -> AppResult<()> {
        let state = {
            let mut registry = self.inner.registry.lock().await;

            let entry = registry
                .items
                .get_mut(id)
                .ok_or_else(|| AppError::invalid("No such download."))?;

            entry.state.priority = priority;
            entry.state.updated_at = now_rfc3339();

            entry.state.clone()
        };

        self.publish(&state);
        self.pump().await;

        Ok(())
    }

    /// Forget every finished row.
    pub async fn clear_finished(&self) -> usize {
        let mut registry = self.inner.registry.lock().await;

        let before = registry.items.len();

        registry.items.retain(|_, e| !e.state.status.is_finished());

        before - registry.items.len()
    }

    /// Cancel everything and forget it. Used on sign-out — a queue is the
    /// previous account's business.
    pub async fn clear_all(&self) {
        let mut registry = self.inner.registry.lock().await;

        for entry in registry.items.values() {
            entry.control.cancel.store(true, Ordering::SeqCst);
        }

        registry.items.clear();
        registry.running = 0;
    }

    // ------------------------------------------------------------ Scheduling

    /// Start whatever can start.
    ///
    /// Called after every state change rather than from a polling loop: the set
    /// of things that can make a download runnable is small and each of them
    /// already has to take the lock.
    async fn pump(&self) {
        loop {
            let next = {
                let mut registry = self.inner.registry.lock().await;

                if registry.running >= registry.max_concurrent {
                    return;
                }

                let pick = registry
                    .items
                    .values()
                    .filter(|e| e.state.status == Status::Queued)
                    .min_by(|a, b| {
                        b.state
                            .priority
                            .cmp(&a.state.priority)
                            .then_with(|| a.seq.cmp(&b.seq))
                    })
                    .map(|e| e.state.id.clone());

                let Some(id) = pick else {
                    let idle = !registry.items.values().any(|e| e.state.status.is_active());

                    if idle && registry.running == 0 {
                        let _ = self.inner.events.send(DownloadEvent::Idle);
                    }

                    return;
                };

                registry.running += 1;

                if let Some(entry) = registry.items.get_mut(&id) {
                    entry.state.status = Status::Running;
                    entry.state.updated_at = now_rfc3339();
                    entry.state.error = None;
                }

                id
            };

            if let Some(state) = self.get(&next).await {
                self.publish(&state);
            }

            let manager = self.clone();

            tokio::spawn(async move {
                manager.run_one(next).await;
            });
        }
    }

    fn publish(&self, state: &DownloadState) {
        // A send with no subscribers is not an error — the app may not have a
        // window open yet.
        let _ = self.inner.events.send(DownloadEvent::Progress {
            download: Box::new(state.clone()),
        });
    }

    async fn run_one(&self, id: String) {
        let outcome = self.transfer(&id).await;

        let state = {
            let mut registry = self.inner.registry.lock().await;

            registry.running = registry.running.saturating_sub(1);

            let Some(entry) = registry.items.get_mut(&id) else {
                return;
            };

            match outcome {
                Ok(Stop::Done) => {
                    entry.state.status = Status::Done;
                    entry.state.error = None;
                    entry.state.speed_bps = 0;
                    entry.state.eta_secs = None;

                    if let Some(total) = entry.state.total {
                        entry.state.done = total;
                    }
                }
                Ok(Stop::Paused) => {
                    entry.state.status = Status::Paused;
                    entry.state.speed_bps = 0;
                    entry.state.eta_secs = None;
                }
                Ok(Stop::Cancelled) => {
                    entry.state.status = Status::Cancelled;
                    entry.state.speed_bps = 0;
                }
                Err(err) => {
                    entry.state.attempts += 1;
                    entry.state.error = Some(err.to_string());
                    entry.state.speed_bps = 0;
                    entry.state.eta_secs = None;

                    /*
                     * A retry goes back to QUEUED rather than starting straight
                     * away, so it takes its turn behind whatever else is
                     * waiting. A permanently-broken URL at the front of a large
                     * queue would otherwise spin through its attempts while
                     * nothing else moved.
                     */
                    entry.state.status = if entry.state.attempts < MAX_ATTEMPTS && err.retryable {
                        Status::Queued
                    } else {
                        Status::Failed
                    };
                }
            }

            entry.state.updated_at = now_rfc3339();

            entry.state.clone()
        };

        if state.status == Status::Failed {
            audit!(
                self.inner.audit,
                Error,
                Network,
                "download.fail",
                format!(
                    "{}: {}",
                    state.label,
                    state.error.clone().unwrap_or_default()
                )
            );
        }

        self.publish(&state);
        self.pump_boxed().await;
    }

    /// [`Self::pump`] with its type erased.
    ///
    /// `pump` spawns `run_one`, and `run_one` pumps again when it finishes —
    /// which is correct behaviour and an infinitely large future type, since
    /// each one would literally contain the other. A `Box<dyn Future>` is a
    /// concrete size, so the cycle stops at the allocation. It happens once per
    /// completed download, which is not a rate worth optimising.
    fn pump_boxed(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let manager = self.clone();

        Box::pin(async move { manager.pump().await })
    }

    /// The transfer itself.
    async fn transfer(&self, id: &str) -> Result<Stop, TransferError> {
        let (mut state, control) = {
            let registry = self.inner.registry.lock().await;

            let entry = registry
                .items
                .get(id)
                .ok_or_else(|| TransferError::fatal("This download is no longer queued."))?;

            (entry.state.clone(), Arc::clone(&entry.control))
        };

        let dest = PathBuf::from(&state.dest);
        let part = part_path(&dest);

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| TransferError::fatal(format!("Could not create the folder: {e}")))?;
        }

        // What is already on disk from a previous attempt.
        let existing = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

        let url = url::Url::parse(&state.url)
            .map_err(|_| TransferError::fatal("That is not a valid download URL."))?;

        audit!(
            self.inner.audit,
            Security,
            Network,
            "download.start",
            format!("{} → {}", state.url, dest.display())
        );

        let mut request = self.inner.http.get(url);

        if existing > 0 {
            request = request.header("range", format!("bytes={existing}-"));
        }

        let response = request
            .send()
            .await
            .map_err(|e| TransferError::retryable(format!("Could not reach the server: {e}")))?;

        let status = response.status();

        if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            // The `.part` is at least as long as the file. Almost always means
            // the file changed; start over rather than keep a stale prefix.
            let _ = std::fs::remove_file(&part);

            return Err(TransferError::retryable(
                "The file on the server changed. Starting again.",
            ));
        }

        if !status.is_success() {
            let message = format!("The server refused the download ({status}).");

            return Err(if status.is_server_error() || status == 429 {
                TransferError::retryable(message)
            } else {
                TransferError::fatal(message)
            });
        }

        /*
         * THE RESUME TRAP.
         *
         * A range request that the server ignores comes back `200` with the
         * WHOLE file. Appending that to what is already on disk produces a file
         * that is too long, passes every length check the caller might make,
         * and fails its checksum with an error nobody can explain. So a `200`
         * in answer to a range request truncates and starts over.
         */
        let resuming = existing > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
        let start_at = if resuming { existing } else { 0 };

        let total = response
            .content_length()
            .map(|len| len + start_at)
            .or(state.total);

        if total.is_some_and(|t| t > MAX_FILE_BYTES) {
            return Err(TransferError::fatal(
                "That file is larger than this app will download.",
            ));
        }

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(!resuming)
            .append(resuming)
            .open(&part)
            .map_err(|e| TransferError::fatal(format!("Could not open the file: {e}")))?;

        state.total = total;
        state.done = start_at;

        let mut stream = response.bytes_stream();

        let mut tick = Instant::now();
        let mut since_tick: u64 = 0;

        while let Some(chunk) = stream.next().await {
            if control.cancel.load(Ordering::SeqCst) {
                drop(file);
                let _ = std::fs::remove_file(&part);

                return Ok(Stop::Cancelled);
            }

            if control.pause.load(Ordering::SeqCst) {
                // The `.part` file stays. That is the whole point.
                self.commit(id, &state).await;

                return Ok(Stop::Paused);
            }

            let chunk = chunk
                .map_err(|e| TransferError::retryable(format!("The transfer stopped: {e}")))?;

            // Both limits, in order. Neither knows about the other, and the
            // effective rate is the lower of the two.
            self.inner.global.take(chunk.len()).await;
            control.limiter.take(chunk.len()).await;

            std::io::Write::write_all(&mut file, &chunk)
                .map_err(|e| TransferError::fatal(format!("Could not write to disk: {e}")))?;

            state.done += chunk.len() as u64;
            since_tick += chunk.len() as u64;

            if state.done > MAX_FILE_BYTES {
                drop(file);
                let _ = std::fs::remove_file(&part);

                return Err(TransferError::fatal(
                    "That file is larger than this app will download.",
                ));
            }

            if tick.elapsed() >= TICK {
                let seconds = tick.elapsed().as_secs_f64().max(0.001);

                state.speed_bps = (since_tick as f64 / seconds) as u64;
                state.eta_secs = eta(state.total, state.done, state.speed_bps);

                state.samples.push(state.speed_bps);

                if state.samples.len() > MAX_SAMPLES {
                    state.samples.remove(0);
                }

                self.commit(id, &state).await;

                tick = Instant::now();
                since_tick = 0;
            }
        }

        drop(file);

        if let Some(expected) = &state.sha256 {
            /*
             * Hashed from the file rather than incrementally as bytes arrive.
             * An incremental hash cannot survive a resume — the bytes from the
             * first attempt never pass through this process — and a checksum
             * that silently stops being checked on resumed downloads is worse
             * than none.
             */
            verify(&part, expected)?;
        }

        /*
         * Rename LAST. Until this line the target name does not exist, so
         * nothing downstream can pick up a partial file and treat it as
         * complete — which is the failure that produces "the mod installed but
         * the game will not start".
         */
        std::fs::rename(&part, &dest)
            .map_err(|e| TransferError::fatal(format!("Could not finish the file: {e}")))?;

        state.done = state.total.unwrap_or(state.done);

        self.commit(id, &state).await;

        audit!(
            self.inner.audit,
            Info,
            Network,
            "download.done",
            format!("{} ({} bytes)", state.label, state.done)
        );

        Ok(Stop::Done)
    }

    /// Copy the working state back into the registry and publish it.
    async fn commit(&self, id: &str, state: &DownloadState) {
        let published = {
            let mut registry = self.inner.registry.lock().await;

            let Some(entry) = registry.items.get_mut(id) else {
                return;
            };

            entry.state.total = state.total;
            entry.state.done = state.done;
            entry.state.speed_bps = state.speed_bps;
            entry.state.eta_secs = state.eta_secs;
            entry.state.samples = state.samples.clone();
            entry.state.updated_at = now_rfc3339();

            entry.state.clone()
        };

        self.publish(&published);
    }
}

/// Has this download finished, and how?
fn terminal(state: &DownloadState) -> Option<AppResult<()>> {
    match state.status {
        Status::Done => Some(Ok(())),
        Status::Failed => Some(Err(AppError::Network(
            state
                .error
                .clone()
                .unwrap_or_else(|| "The download failed.".into()),
        ))),
        Status::Cancelled => Some(Err(AppError::invalid("The download was cancelled."))),
        Status::Paused => Some(Err(AppError::invalid(
            "The download is paused. Resume it to continue installing.",
        ))),
        Status::Queued | Status::Running => None,
    }
}

/// Why a transfer stopped without failing.
enum Stop {
    Done,
    Paused,
    Cancelled,
}

/// A failure, and whether trying again could help.
struct TransferError {
    message: String,
    retryable: bool,
}

impl TransferError {
    fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn eta(total: Option<u64>, done: u64, speed: u64) -> Option<u64> {
    let total = total?;

    if speed == 0 || done >= total {
        return None;
    }

    Some((total - done) / speed.max(1))
}

fn rank(status: Status) -> u8 {
    match status {
        Status::Running => 0,
        Status::Queued => 1,
        Status::Paused => 2,
        Status::Failed => 3,
        Status::Done => 4,
        Status::Cancelled => 5,
    }
}

/// `foo.zip` → `foo.zip.tmcpart`.
///
/// A distinctive suffix rather than `.part`: several other downloaders use
/// `.part`, and a user with both pointed at one folder should not have them
/// fighting over the same temporary name.
fn part_path(dest: &std::path::Path) -> PathBuf {
    let mut name = dest
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();

    name.push(".tmcpart");

    dest.with_file_name(name)
}

fn verify(path: &std::path::Path, expected: &str) -> Result<(), TransferError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| TransferError::fatal(format!("Could not read the file back: {e}")))?;

    let mut hasher = Sha256::new();

    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| TransferError::fatal(format!("Could not read the file back: {e}")))?;

    let actual = hex::encode(hasher.finalize());

    if !actual.eq_ignore_ascii_case(expected.trim()) {
        drop(file);

        // Delete it. A file that failed its checksum must not be left where a
        // later step could pick it up — the same rule the plugin executor's
        // download step follows.
        let _ = std::fs::remove_file(path);

        return Err(TransferError::fatal(
            "The download did not match its expected checksum.",
        ));
    }

    Ok(())
}

/// The checks this module makes for itself.
fn validate(request: &DownloadRequest) -> AppResult<()> {
    if request.id.is_empty() || request.id.len() > 128 {
        return Err(AppError::invalid("A download needs a short, stable id."));
    }

    let url = url::Url::parse(&request.url)
        .map_err(|_| AppError::invalid("That is not a valid download URL."))?;

    // Plaintext would let anyone on the path replace a mod archive with
    // anything they liked — checksum included, since it travels the same way.
    if url.scheme() != "https" {
        return Err(AppError::invalid("Downloads must use https."));
    }

    let host = url
        .host_str()
        .ok_or_else(|| AppError::invalid("That download URL has no host."))?;

    /*
     * A literal private address is refused outright. A HOSTNAME that resolves
     * to one is not caught here — `reqwest` does its own resolution and this
     * crate cannot pin the address it picks — so this is a cheap first line
     * rather than the whole guard. What actually bounds the damage is that the
     * caller decides the URL: it comes from the API or from a plugin manifest's
     * host allow-list, never from the webview.
     */
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if !net::addr::is_public(&ip) {
            return Err(AppError::jail("That download address is not a public one."));
        }
    }

    if !request.dest.is_absolute() {
        return Err(AppError::invalid(
            "A download needs an absolute destination.",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // ------------------------------------------------------- A server to hit
    //
    // A real transfer is the only way to test resume, the range trap and the
    // checksum path — all three are about what a SERVER does, and a mock of the
    // client side would only ever confirm what the mock was told to say.
    //
    // Twenty lines of HTTP/1.1 is cheaper than a dependency and cannot drift
    // from what this module actually sends.

    struct TestServer {
        port: u16,
    }

    /// What the server should do, so one harness covers every case.
    #[derive(Clone, Copy)]
    enum Behaviour {
        /// Honour `Range` properly.
        Ranges,
        /// Ignore `Range` and send the whole body with `200` — the trap.
        IgnoreRange,
        /// Close the connection after `n` bytes, once.
        TruncateOnce(usize),
        Status(u16),
    }

    async fn serve(body: Vec<u8>, behaviour: Behaviour) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("addr").port();

        tokio::spawn(async move {
            let mut truncated = false;

            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };

                // Read the request head. Every request this module sends is a
                // GET with a handful of headers, so one buffer is enough.
                let mut buf = vec![0u8; 4096];
                let read = socket.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..read]).to_string();

                let start: u64 = head
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("range:"))
                    .and_then(|l| l.split('=').nth(1))
                    .and_then(|r| r.trim_end_matches('-').trim().parse().ok())
                    .unwrap_or(0);

                let response = match behaviour {
                    Behaviour::Status(code) => format!(
                        "HTTP/1.1 {code} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    )
                    .into_bytes(),
                    Behaviour::IgnoreRange => {
                        let mut out = format!(
                            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();

                        out.extend_from_slice(&body);
                        out
                    }
                    Behaviour::Ranges | Behaviour::TruncateOnce(_) => {
                        let slice = &body[(start as usize).min(body.len())..];

                        let mut out = if start > 0 {
                            format!(
                                "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\n\
                                 content-range: bytes {}-{}/{}\r\nconnection: close\r\n\r\n",
                                slice.len(),
                                start,
                                body.len().saturating_sub(1),
                                body.len()
                            )
                        } else {
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\
                                 accept-ranges: bytes\r\nconnection: close\r\n\r\n",
                                slice.len()
                            )
                        }
                        .into_bytes();

                        let send = match behaviour {
                            Behaviour::TruncateOnce(n) if !truncated => {
                                truncated = true;
                                &slice[..n.min(slice.len())]
                            }
                            _ => slice,
                        };

                        out.extend_from_slice(send);
                        out
                    }
                };

                let _ = socket.write_all(&response).await;
                let _ = socket.flush().await;
                // Dropping the socket is what tells the client the body ended
                // when it was truncated.
            }
        });

        TestServer { port }
    }

    fn manager() -> DownloadManager {
        let tmp = std::env::temp_dir().join(format!("tmc-dl-{}.jsonl", std::process::id()));

        DownloadManager::new(reqwest::Client::new(), Arc::new(Audit::new(tmp)))
    }

    /// Queue a download, bypassing [`validate`]'s https rule.
    ///
    /// The rule is right and is tested separately; a loopback HTTP server is
    /// the only way to exercise the transfer itself, and adding a runtime
    /// exemption for it would put a plaintext hole in shipping code so that a
    /// test could run.
    async fn queue_raw(manager: &DownloadManager, request: DownloadRequest) {
        let now = now_rfc3339();

        let state = DownloadState {
            id: request.id.clone(),
            label: request.label.clone(),
            url: request.url.clone(),
            dest: request.dest.to_string_lossy().into_owned(),
            status: Status::Queued,
            total: request.size_hint,
            done: 0,
            speed_bps: 0,
            eta_secs: None,
            priority: request.priority,
            limit_bps: request.limit_bps,
            attempts: 0,
            error: None,
            sha256: request.sha256.clone(),
            samples: Vec::new(),
            queued_at: now.clone(),
            updated_at: now,
            meta: request.meta.clone(),
        };

        {
            let mut registry = manager.inner.registry.lock().await;

            registry.seq += 1;
            let seq = registry.seq;

            registry.items.insert(
                request.id.clone(),
                Entry {
                    state,
                    control: Arc::new(Control {
                        cancel: AtomicBool::new(false),
                        pause: AtomicBool::new(false),
                        limiter: Limiter::new(request.limit_bps),
                    }),
                    seq,
                },
            );
        }

        manager.pump().await;
    }

    async fn wait_for(manager: &DownloadManager, id: &str, want: Status) -> DownloadState {
        for _ in 0..600 {
            if let Some(state) = manager.get(id).await {
                if state.status == want {
                    return state;
                }
            }

            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        panic!(
            "{id} never reached {want:?}; it is {:?}",
            manager.get(id).await.map(|s| s.status)
        );
    }

    fn request(id: &str, port: u16, dest: PathBuf) -> DownloadRequest {
        DownloadRequest {
            id: id.into(),
            url: format!("http://127.0.0.1:{port}/file.bin"),
            dest,
            label: "Test file".into(),
            sha256: None,
            size_hint: None,
            priority: 0,
            limit_bps: None,
            meta: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn a_download_lands_at_its_destination_and_not_before() {
        let body = vec![7u8; 64 * 1024];
        let server = serve(body.clone(), Behaviour::Ranges).await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("out/file.bin");

        let manager = manager();

        queue_raw(&manager, request("d1", server.port, dest.clone())).await;

        let state = wait_for(&manager, "d1", Status::Done).await;

        assert_eq!(state.done, body.len() as u64);
        assert_eq!(std::fs::read(&dest).expect("read"), body);
        assert!(
            !part_path(&dest).exists(),
            "the partial file must not survive a completed download"
        );
    }

    /// The whole point of the queue. A `.part` from a previous attempt must be
    /// continued, not re-fetched.
    #[tokio::test]
    async fn a_resumed_download_asks_only_for_the_rest() {
        let body: Vec<u8> = (0..40_000u32).map(|n| n as u8).collect();
        let server = serve(body.clone(), Behaviour::Ranges).await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("file.bin");

        // Half of it, already on disk from an earlier run.
        std::fs::write(part_path(&dest), &body[..20_000]).expect("write part");

        let manager = manager();

        queue_raw(&manager, request("d1", server.port, dest.clone())).await;

        let state = wait_for(&manager, "d1", Status::Done).await;

        assert_eq!(
            std::fs::read(&dest).expect("read"),
            body,
            "a resume must reconstruct the file exactly"
        );

        assert_eq!(state.done, body.len() as u64);
    }

    /// A server that ignores `Range` answers `200` with the WHOLE file.
    /// Appending that to what is on disk gives a file that is too long and
    /// fails its checksum with an error nobody can explain.
    #[tokio::test]
    async fn a_server_that_ignores_range_makes_the_file_restart() {
        let body: Vec<u8> = (0..30_000u32).map(|n| (n % 251) as u8).collect();
        let server = serve(body.clone(), Behaviour::IgnoreRange).await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("file.bin");

        std::fs::write(part_path(&dest), &body[..10_000]).expect("write part");

        let manager = manager();

        queue_raw(&manager, request("d1", server.port, dest.clone())).await;

        wait_for(&manager, "d1", Status::Done).await;

        assert_eq!(
            std::fs::read(&dest).expect("read").len(),
            body.len(),
            "the partial prefix must have been discarded, not prepended"
        );
        assert_eq!(std::fs::read(&dest).expect("read"), body);
    }

    #[tokio::test]
    async fn a_bad_checksum_deletes_the_file_rather_than_installing_it() {
        let body = vec![1u8; 4096];
        let server = serve(body, Behaviour::Ranges).await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("file.bin");

        let mut req = request("d1", server.port, dest.clone());
        req.sha256 = Some("00".repeat(32));

        let manager = manager();

        queue_raw(&manager, req).await;

        let state = wait_for(&manager, "d1", Status::Failed).await;

        assert!(state.error.unwrap_or_default().contains("checksum"));
        assert!(
            !dest.exists(),
            "a file that failed its checksum must not be left"
        );
        assert!(!part_path(&dest).exists());
    }

    #[tokio::test]
    async fn a_correct_checksum_passes() {
        let body = vec![1u8; 4096];
        let digest = hex::encode(Sha256::digest(&body));

        let server = serve(body, Behaviour::Ranges).await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("file.bin");

        let mut req = request("d1", server.port, dest.clone());
        req.sha256 = Some(digest);

        let manager = manager();

        queue_raw(&manager, req).await;
        wait_for(&manager, "d1", Status::Done).await;

        assert!(dest.exists());
    }

    /// A connection that drops mid-file is the ordinary case on a phone. It
    /// must retry by itself, and the retry must not start from zero.
    #[tokio::test]
    async fn a_dropped_connection_is_retried_and_resumes() {
        let body: Vec<u8> = (0..50_000u32).map(|n| (n % 253) as u8).collect();
        let server = serve(body.clone(), Behaviour::TruncateOnce(10_000)).await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("file.bin");

        let manager = manager();

        queue_raw(&manager, request("d1", server.port, dest.clone())).await;

        let state = wait_for(&manager, "d1", Status::Done).await;

        assert_eq!(std::fs::read(&dest).expect("read"), body);
        assert!(
            state.attempts >= 1,
            "the first attempt was cut short, so there must have been a second"
        );
    }

    #[tokio::test]
    async fn a_404_fails_immediately_rather_than_retrying() {
        let server = serve(Vec::new(), Behaviour::Status(404)).await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("file.bin");

        let manager = manager();

        queue_raw(&manager, request("d1", server.port, dest)).await;

        let state = wait_for(&manager, "d1", Status::Failed).await;

        assert_eq!(
            state.attempts, 1,
            "a 404 will not become a 200; retrying it wastes the user's time"
        );
    }

    #[tokio::test]
    async fn cancelling_removes_the_partial_file() {
        let body = vec![3u8; 8 * 1024 * 1024];
        let server = serve(body, Behaviour::Ranges).await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("file.bin");

        let manager = manager();

        let mut req = request("d1", server.port, dest.clone());
        // Slow enough that the cancel lands mid-transfer.
        req.limit_bps = Some(64 * 1024);

        queue_raw(&manager, req).await;

        tokio::time::sleep(Duration::from_millis(300)).await;

        manager.cancel("d1").await.expect("cancel");

        wait_for(&manager, "d1", Status::Cancelled).await;

        assert!(!dest.exists());
        assert!(!part_path(&dest).exists(), "a cancel throws the bytes away");
    }

    #[tokio::test]
    async fn pausing_keeps_the_partial_file_and_resuming_finishes_it() {
        let body: Vec<u8> = (0..2_000_000u32).map(|n| (n % 251) as u8).collect();
        let server = serve(body.clone(), Behaviour::Ranges).await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let dest = tmp.path().join("file.bin");

        let manager = manager();

        let mut req = request("d1", server.port, dest.clone());
        req.limit_bps = Some(256 * 1024);

        queue_raw(&manager, req).await;

        tokio::time::sleep(Duration::from_millis(400)).await;

        manager.pause("d1").await.expect("pause");

        let paused = wait_for(&manager, "d1", Status::Paused).await;

        assert!(paused.done > 0, "a pause has to keep what it had");
        assert!(
            part_path(&dest).exists(),
            "a pause keeps the partial file — that is the whole point"
        );
        assert!(!dest.exists(), "and does not produce the real one");

        manager.set_limit("d1", None).await.expect("unthrottle");
        manager.resume("d1").await.expect("resume");

        wait_for(&manager, "d1", Status::Done).await;

        assert_eq!(std::fs::read(&dest).expect("read"), body);
    }

    #[tokio::test]
    async fn the_highest_priority_queued_download_runs_first() {
        let body = vec![9u8; 1024];
        let server = serve(body, Behaviour::Ranges).await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let manager = manager();

        // One at a time, so the order is observable.
        manager.set_concurrency(1).await;

        for (id, priority) in [("low", 1), ("high", 9), ("mid", 5)] {
            let mut req = request(id, server.port, tmp.path().join(format!("{id}.bin")));
            req.priority = priority;
            req.limit_bps = Some(16 * 1024);

            queue_raw(&manager, req).await;
        }

        for id in ["high", "mid", "low"] {
            wait_for(&manager, id, Status::Done).await;
        }

        // All three finished; the list orders them the way the queue does.
        let listed = manager.list().await;

        assert_eq!(listed.len(), 3);
    }

    // ----------------------------------------------------------- Pure checks

    #[tokio::test]
    async fn enqueue_refuses_what_it_should() {
        let manager = manager();

        let base = DownloadRequest {
            id: "d1".into(),
            url: "https://cdn.test/a.jar".into(),
            dest: PathBuf::from("/tmp/a.jar"),
            label: "A".into(),
            sha256: None,
            size_hint: None,
            priority: 0,
            limit_bps: None,
            meta: BTreeMap::new(),
        };

        // Plaintext would let anyone on the path swap the file, checksum and
        // all — it travels the same way.
        let mut http = base.clone();
        http.url = "http://cdn.test/a.jar".into();
        assert!(manager.enqueue(http).await.is_err());

        // A private address is the SSRF shape.
        let mut private = base.clone();
        private.url = "https://192.168.1.1/a.jar".into();
        assert!(manager.enqueue(private).await.is_err());

        let mut loopback = base.clone();
        loopback.url = "https://127.0.0.1/a.jar".into();
        assert!(manager.enqueue(loopback).await.is_err());

        // A relative destination would resolve against a working directory
        // that means nothing for a GUI app.
        let mut relative = base.clone();
        relative.dest = PathBuf::from("a.jar");
        assert!(manager.enqueue(relative).await.is_err());

        let mut no_id = base.clone();
        no_id.id = String::new();
        assert!(manager.enqueue(no_id).await.is_err());
    }

    #[tokio::test]
    async fn enqueuing_the_same_id_twice_does_not_produce_two_writers() {
        let manager = manager();

        let request = DownloadRequest {
            id: "d1".into(),
            url: "https://cdn.test/a.jar".into(),
            dest: PathBuf::from("/tmp/tmc-test/a.jar"),
            label: "A".into(),
            sha256: None,
            size_hint: None,
            priority: 0,
            limit_bps: None,
            meta: BTreeMap::new(),
        };

        manager.enqueue(request.clone()).await.expect("first");
        manager.enqueue(request).await.expect("second");

        assert_eq!(manager.list().await.len(), 1);
    }

    #[test]
    fn the_partial_file_has_a_name_of_our_own() {
        let part = part_path(std::path::Path::new("/games/mods/cool.jar"));

        assert_eq!(part, PathBuf::from("/games/mods/cool.jar.tmcpart"));
    }

    #[test]
    fn an_eta_needs_both_a_total_and_a_speed() {
        assert_eq!(eta(Some(1000), 500, 100), Some(5));
        assert_eq!(eta(None, 500, 100), None);
        assert_eq!(eta(Some(1000), 500, 0), None);
        // Already there.
        assert_eq!(eta(Some(1000), 1000, 100), None);
    }

    #[test]
    fn percent_is_bounded_and_declines_to_guess() {
        let mut state = DownloadState {
            id: "d".into(),
            label: String::new(),
            url: String::new(),
            dest: String::new(),
            status: Status::Running,
            total: Some(200),
            done: 50,
            speed_bps: 0,
            eta_secs: None,
            priority: 0,
            limit_bps: None,
            attempts: 0,
            error: None,
            sha256: None,
            samples: Vec::new(),
            queued_at: String::new(),
            updated_at: String::new(),
            meta: BTreeMap::new(),
        };

        assert_eq!(state.percent(), Some(25.0));

        state.total = None;
        assert_eq!(state.percent(), None);

        state.total = Some(0);
        assert_eq!(state.percent(), None);
    }
}
