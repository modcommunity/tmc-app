//! **A game that is running**, and how long it ran for.
//!
//! The app could start a game before this module existed; it could not tell you
//! anything about one afterwards. That gap was load-bearing in three places at
//! once — the account's `playSeconds` was reported as a literal `0` on every
//! launch, the Library could not say whether a game was already open, and a
//! game that died on startup left nothing behind to read. All three are the
//! same missing fact: nobody kept the [`std::process::Child`].
//!
//! WHAT A SESSION IS, AND WHAT IT IS NOT
//! ------------------------------------
//! A session is one attempt to start one thing. It is deliberately NOT "one
//! period of play" — those are the same only when the app is the thing that
//! started the process, and [`SessionKind`] is the honest record of when it was
//! not:
//!
//!   * [`SessionKind::Process`] — we spawned it, we hold the handle, we know
//!     when it exited and with what. Playtime here is measured.
//!   * [`SessionKind::Handoff`] — the plan was a `steam://` URI, so the OS
//!     opener started a launcher which started the game. There is no child, no
//!     exit code, and **no playtime**: the process we could have timed belongs
//!     to Valve. Recording zero here rather than guessing is the point — a
//!     launcher that invents a play session from a URI it fired is a launcher
//!     whose statistics are fiction.
//!   * [`SessionKind::Web`] — the browser player. It ends when the window
//!     closes, which the app does observe, so this one is measured too.
//!
//! **A duration is never inferred from a later launch.** Both of the other mod
//! managers studied for this either treat "the app was closed" as the end of a
//! session or clamp a long one to some plausible number. Both produce a
//! plausible-looking figure that is wrong, and a wrong number nobody can
//! distinguish from a right one is worse than an absent one. An unfinished
//! session simply has no `ended_ms` until it ends.
//!
//! CAPTURING WHAT THE GAME SAID
//! ---------------------------
//! A supervised process gets its stdout and stderr written to one file per
//! session. That is the whole crash-report story and it is worth the pipe: a
//! game that exits in two seconds has almost always printed the reason, and
//! without a pipe that reason goes to a console nobody attached.
//!
//! The capture is **bounded** ([`MAX_LOG_BYTES`]). A game left running
//! overnight with a chatty logger is the normal case, not an edge case, and an
//! unbounded capture is a disk-filling bug that only shows up on the machines
//! of the people who use the feature most.
//!
//! WHY THE SUPERVISOR IS A THREAD AND NOT A TASK
//! --------------------------------------------
//! `Child::wait` blocks, and this crate does not own the runtime — the app
//! crate does, and `tmc-core` is compiled and tested without one. One
//! `std::thread` per running game is the right cost for something whose upper
//! bound is "how many games can this machine run at once".

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// The most one session's captured output may occupy on disk.
///
/// 4 MiB holds a Minecraft crash report and its whole mod list several times
/// over, which is the case this exists for. Past it the capture stops and says
/// so in the file rather than rotating: a rotated log loses the START of the
/// output, and the start is where a startup failure is described.
pub const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// How many finished sessions are kept in memory for the UI to list.
///
/// The durable history lives in the library database; this is only what the
/// Library screen shows without a query. Bounded because a long-lived desktop
/// process would otherwise accumulate one entry per launch forever.
const MAX_RECENT: usize = 100;

/// How a session was started, which decides what can be known about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionKind {
    /// We spawned it and hold the handle. Duration and exit code are real.
    Process,
    /// A `steam://`-style URI went to the OS opener. Nothing further is known.
    Handoff,
    /// The in-app web player. Ends when its window closes.
    Web,
}

impl SessionKind {
    /// Whether a duration measured for this kind means anything.
    ///
    /// The one caller that matters is the playtime report: a handoff must
    /// contribute nothing rather than contribute the time between firing a URI
    /// and the app noticing, which is a number about the app.
    pub fn measurable(self) -> bool {
        matches!(self, Self::Process | Self::Web)
    }
}

/// What the app knows about one launch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: u64,
    pub kind: SessionKind,
    /// The TMC app id, when the launch was for a known game.
    pub app_id: Option<i64>,
    /// The game's plugin slug, for artwork and rules.
    pub app_slug: Option<String>,
    /// What to call this on screen — the sandbox's name, or the game's.
    pub label: String,
    /// The sandbox this was launched from, when it was launched from one.
    pub sandbox_id: Option<i64>,
    /// The cloud install id, when the sandbox has one. What playtime reports
    /// against.
    pub install_id: Option<i64>,
    pub started_ms: i64,
    /// Absent while the session is still running.
    pub ended_ms: Option<i64>,
    /// The process id, for a supervised launch. Informational only — nothing
    /// here signals by pid, because a pid is reused.
    pub pid: Option<u32>,
    /// The child's exit status, once it has one.
    pub exit_code: Option<i32>,
    /// Set when the app itself ended the session.
    pub stopped_by_user: bool,
    /// Where the captured output went, when there is any.
    pub log_path: Option<String>,
}

impl Session {
    /// Seconds elapsed, or `None` for a session whose duration means nothing.
    ///
    /// A running session reports the time so far, which is what the Library's
    /// "playing now" line shows. A [`SessionKind::Handoff`] reports `None`
    /// however long it has been open — see the module header.
    pub fn seconds(&self, now_ms: i64) -> Option<i64> {
        if !self.kind.measurable() {
            return None;
        }

        let end = self.ended_ms.unwrap_or(now_ms);

        Some(((end - self.started_ms).max(0)) / 1000)
    }

    pub fn running(&self) -> bool {
        self.ended_ms.is_none()
    }
}

/// What a caller says about a launch before it starts.
#[derive(Debug, Clone, Default)]
pub struct SessionSpec {
    pub app_id: Option<i64>,
    pub app_slug: Option<String>,
    pub label: String,
    pub sandbox_id: Option<i64>,
    pub install_id: Option<i64>,
}

/// A finished session, handed to whoever is interested.
///
/// A callback rather than a database handle inside this module: persisting a
/// session and reporting playtime to the account are the app crate's business
/// and both can fail in ways a supervisor thread has nowhere to report.
pub type OnEnd = Arc<dyn Fn(&Session) + Send + Sync>;

struct Live {
    session: Session,
    /// `None` for a handoff or a web session — there is nothing to signal.
    child: Option<Arc<Mutex<Child>>>,
}

/// Every session this process has started, live and recent.
pub struct Sessions {
    inner: Mutex<State>,
    next_id: AtomicU64,
    /// Where per-session output files are written.
    log_dir: PathBuf,
    on_end: Mutex<Option<OnEnd>>,
}

#[derive(Default)]
struct State {
    live: BTreeMap<u64, Live>,
    recent: Vec<Session>,
}

impl Sessions {
    pub fn new(log_dir: PathBuf) -> Self {
        Self {
            inner: Mutex::new(State::default()),
            next_id: AtomicU64::new(1),
            log_dir,
            on_end: Mutex::new(None),
        }
    }

    /// Register what happens when a session finishes.
    ///
    /// Set once, during state assembly. Replacing it later would silently drop
    /// the end of any session already being supervised against the old one.
    pub fn on_end(&self, hook: OnEnd) {
        if let Ok(mut slot) = self.on_end.lock() {
            *slot = Some(hook);
        }
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, State>> {
        self.inner
            .lock()
            .map_err(|_| AppError::internal("session registry lock poisoned"))
    }

    /// Everything running right now, oldest first.
    pub fn running(&self) -> Vec<Session> {
        self.lock()
            .map(|state| state.live.values().map(|l| l.session.clone()).collect())
            .unwrap_or_default()
    }

    /// The last [`MAX_RECENT`] finished sessions, newest first.
    pub fn recent(&self) -> Vec<Session> {
        self.lock()
            .map(|state| {
                let mut out = state.recent.clone();

                out.reverse();
                out
            })
            .unwrap_or_default()
    }

    /// Is anything running for this game?
    pub fn running_for_app(&self, app_id: i64) -> bool {
        self.lock()
            .map(|state| {
                state
                    .live
                    .values()
                    .any(|l| l.session.app_id == Some(app_id))
            })
            .unwrap_or(false)
    }

    /// Is anything running for this sandbox?
    ///
    /// The one check that gates a deploy: writing into a game folder while the
    /// game has those files open fails on Windows and corrupts on nobody's
    /// schedule elsewhere.
    pub fn running_for_sandbox(&self, sandbox_id: i64) -> bool {
        self.lock()
            .map(|state| {
                state
                    .live
                    .values()
                    .any(|l| l.session.sandbox_id == Some(sandbox_id))
            })
            .unwrap_or(false)
    }

    fn next(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Record a launch nothing can be measured about.
    ///
    /// Opens and closes in one call: a handoff has no observable end, so
    /// leaving it "running" would put a game in the Library's running list
    /// forever and block every deploy for that sandbox.
    pub fn record_handoff(&self, spec: SessionSpec) -> Session {
        let now = now_ms();

        let session = Session {
            id: self.next(),
            kind: SessionKind::Handoff,
            app_id: spec.app_id,
            app_slug: spec.app_slug,
            label: spec.label,
            sandbox_id: spec.sandbox_id,
            install_id: spec.install_id,
            started_ms: now,
            ended_ms: Some(now),
            pid: None,
            exit_code: None,
            stopped_by_user: false,
            log_path: None,
        };

        self.finish(session.clone());

        session
    }

    /// Open a session the caller will close itself.
    ///
    /// Used by the web player, whose end is a window closing rather than a
    /// process exiting.
    pub fn open_web(&self, spec: SessionSpec) -> Session {
        let session = Session {
            id: self.next(),
            kind: SessionKind::Web,
            app_id: spec.app_id,
            app_slug: spec.app_slug,
            label: spec.label,
            sandbox_id: spec.sandbox_id,
            install_id: spec.install_id,
            started_ms: now_ms(),
            ended_ms: None,
            pid: None,
            exit_code: None,
            stopped_by_user: false,
            log_path: None,
        };

        if let Ok(mut state) = self.lock() {
            state.live.insert(
                session.id,
                Live {
                    session: session.clone(),
                    child: None,
                },
            );
        }

        session
    }

    /// Spawn a command and watch it until it exits.
    ///
    /// The command arrives fully built — program, argv, cwd and environment —
    /// because deciding any of those is [`crate::launch`]'s job and this must
    /// not become a second place one can be chosen.
    pub fn spawn(self: &Arc<Self>, mut command: Command, spec: SessionSpec) -> AppResult<Session> {
        let id = self.next();
        let log_path = self.log_dir.join(format!("session-{id}.log"));

        /*
         * The log directory is created here rather than at startup: a build that
         * never launches a game should not leave an empty folder in the user's
         * data directory, and a folder deleted between launches has to work.
         */
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let captured = std::fs::File::create(&log_path).ok();

        if captured.is_some() {
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
        } else {
            /*
             * Without somewhere to put it, the output is discarded rather than
             * inherited. An inherited pipe ties the game's lifetime to a console
             * this process may not have, and on Windows it opens one.
             */
            command.stdout(Stdio::null()).stderr(Stdio::null());
        }

        command.stdin(Stdio::null());

        let mut child = command
            .spawn()
            .map_err(|e| AppError::internal(format!("could not start the game: {e}")))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let session = Session {
            id,
            kind: SessionKind::Process,
            app_id: spec.app_id,
            app_slug: spec.app_slug,
            label: spec.label,
            sandbox_id: spec.sandbox_id,
            install_id: spec.install_id,
            started_ms: now_ms(),
            ended_ms: None,
            pid: Some(child.id()),
            exit_code: None,
            stopped_by_user: false,
            log_path: captured
                .is_some()
                .then(|| log_path.to_string_lossy().into_owned()),
        };

        let handle = Arc::new(Mutex::new(child));

        if let Ok(mut state) = self.lock() {
            state.live.insert(
                id,
                Live {
                    session: session.clone(),
                    child: Some(Arc::clone(&handle)),
                },
            );
        }

        if let Some(file) = captured {
            spawn_capture(file, stdout, stderr);
        }

        self.supervise(id, handle);

        Ok(session)
    }

    /// Wait for a child, off the caller's thread, and close its session.
    ///
    /// The thread holds an `Arc<Sessions>` rather than a borrow, which is why
    /// [`Sessions::spawn`] takes `Arc<Self>`. The registry does live for the
    /// whole process in practice, but "in practice immortal" is not a lifetime,
    /// and the alternative — passing a pointer across the thread boundary — is
    /// unsound the first time a test drops one.
    fn supervise(self: &Arc<Self>, id: u64, child: Arc<Mutex<Child>>) {
        let sessions = Arc::clone(self);

        std::thread::spawn(move || {
            /*
             * The lock is taken for the whole wait, and that is correct rather
             * than merely convenient: the only other caller is `stop`, which
             * wants to signal THIS child, and `kill` on a process that has
             * already been reaped is an error about a pid that may since have
             * been reused. Holding it means a stop either lands before the exit
             * or finds the session gone.
             *
             * A poisoned lock leaves the session running, which is honest: we
             * no longer know what the process is doing.
             */
            let status = {
                let Ok(mut guard) = child.lock() else { return };

                guard.wait()
            };

            let Some(mut live) = sessions.lock().ok().and_then(|mut s| s.live.remove(&id)) else {
                // Already ended — a `stop` on a web session, or a second exit
                // notification. Nothing to close.
                return;
            };

            live.session.ended_ms = Some(now_ms());
            live.session.exit_code = status.ok().and_then(|s| s.code());

            sessions.finish(live.session);
        });
    }

    /// Close a session the caller opened, or one it wants stopped.
    ///
    /// Idempotent: a web window closing twice, or a stop racing an exit, must
    /// not produce two end records — the second call finds nothing live and
    /// does nothing.
    pub fn end(&self, id: u64, stopped_by_user: bool) -> Option<Session> {
        let live = { self.lock().ok()?.live.remove(&id)? };

        let mut session = live.session;

        session.ended_ms = Some(now_ms());
        session.stopped_by_user = stopped_by_user;

        self.finish(session.clone());

        Some(session)
    }

    /// Ask a running game to exit.
    ///
    /// `kill`, not a graceful signal, and the difference is worth stating: there
    /// is no portable way to ask a Windows GUI process to close politely, and a
    /// half-portable one that works on two of five platforms is a button that
    /// behaves differently depending on where somebody is sitting. The session
    /// is closed by the supervisor when the process actually goes, not here —
    /// so a kill that fails leaves the session running, which is true.
    pub fn stop(&self, id: u64) -> AppResult<()> {
        let (child, pid) = {
            let mut state = self.lock()?;

            let Some(live) = state.live.get_mut(&id) else {
                return Err(AppError::invalid("That session is not running."));
            };

            /*
             * Marked BEFORE the signal goes out, because after it the
             * supervisor owns the record and this thread never touches it
             * again.
             *
             * Without this every Stop looked like a crash. SIGKILL leaves no
             * exit code, so the history rendered "ended, no exit code" under a
             * red warning triangle — telling somebody their game died when they
             * are the one who closed it.
             */
            live.session.stopped_by_user = true;

            let pid = live.session.pid;

            let Some(child) = live.child.clone() else {
                // A web session has no process; closing its window is the
                // caller's job and `end` is how it reports that.
                drop(state);

                self.end(id, true);

                return Ok(());
            };

            (child, pid)
        };

        /*
         * `Child::kill` needs `&mut Child`, and the supervisor holds that lock
         * for the entire `wait`. So the pid is taken instead and the signal is
         * sent to it directly.
         *
         * Signalling by pid is normally a bug — a pid is reused, so the target
         * may be a stranger's process by the time the signal lands. It is safe
         * HERE and only here: the supervisor is inside `wait` on this child, so
         * the process has not been reaped, so its pid cannot yet have been
         * handed to anybody else. That is the whole reason this does not simply
         * take the lock.
         */
        let Some(pid) = pid else {
            return Err(AppError::invalid("That session has no process to stop."));
        };

        let _ = child;

        kill_pid(pid)
    }

    /// Move a finished session into the recent list and tell the hook.
    fn finish(&self, session: Session) {
        if let Ok(mut state) = self.lock() {
            state.recent.push(session.clone());

            let overflow = state.recent.len().saturating_sub(MAX_RECENT);

            if overflow > 0 {
                state.recent.drain(0..overflow);
            }
        }

        let hook = self.on_end.lock().ok().and_then(|h| h.clone());

        if let Some(hook) = hook {
            hook(&session);
        }
    }

    /// Read back what a session's process printed.
    ///
    /// Bounded by line count from the END, because the interesting part of a
    /// crash is the last thing said before it.
    pub fn log_tail(&self, id: u64, lines: usize) -> AppResult<String> {
        let path = {
            let state = self.lock()?;

            state
                .live
                .get(&id)
                .map(|l| l.session.log_path.clone())
                .or_else(|| {
                    state
                        .recent
                        .iter()
                        .rev()
                        .find(|s| s.id == id)
                        .map(|s| s.log_path.clone())
                })
                .flatten()
        };

        let Some(path) = path else {
            return Ok(String::new());
        };

        tail_file(Path::new(&path), lines)
    }
}

/// The last `lines` lines of a file, or as many as it has.
fn tail_file(path: &Path, lines: usize) -> AppResult<String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        // A session whose log was cleaned up is not an error — it is a session
        // with nothing to show, which is what an empty string says.
        Err(_) => return Ok(String::new()),
    };

    let mut kept: std::collections::VecDeque<String> = std::collections::VecDeque::new();

    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };

        if kept.len() >= lines {
            kept.pop_front();
        }

        kept.push_back(line);
    }

    Ok(kept.into_iter().collect::<Vec<_>>().join("\n"))
}

/// Drain the child's two pipes into one file, interleaved as they arrive.
///
/// One thread per pipe rather than a select loop: two blocking readers is the
/// portable shape, and a game that only ever writes to stderr must not have its
/// output held up behind an stdout read that will never return.
fn spawn_capture(
    file: std::fs::File,
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
) {
    let shared = Arc::new(Mutex::new(Capture {
        file,
        written: 0,
        stopped: false,
    }));

    if let Some(out) = stdout {
        let sink = Arc::clone(&shared);

        std::thread::spawn(move || pump(BufReader::new(out), sink));
    }

    if let Some(err) = stderr {
        let sink = Arc::clone(&shared);

        std::thread::spawn(move || pump(BufReader::new(err), sink));
    }
}

struct Capture {
    file: std::fs::File,
    written: u64,
    stopped: bool,
}

fn pump<R: std::io::Read>(reader: BufReader<R>, sink: Arc<Mutex<Capture>>) {
    for line in reader.lines() {
        let Ok(line) = line else { break };

        let Ok(mut cap) = sink.lock() else { break };

        if cap.stopped {
            break;
        }

        let bytes = line.len() as u64 + 1;

        if cap.written + bytes > MAX_LOG_BYTES {
            let _ = writeln!(
                cap.file,
                "\n[tmc] Output capture stopped at {MAX_LOG_BYTES} bytes."
            );

            cap.stopped = true;

            break;
        }

        if writeln!(cap.file, "{line}").is_err() {
            break;
        }

        cap.written += bytes;
    }
}

/// Milliseconds since the epoch.
///
/// Saturating rather than unwrapping: a device whose clock is before 1970 is a
/// real thing on a phone with a dead battery, and it must not panic a launch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Send a kill signal to a process by id.
///
/// Split by platform because there is no portable way to do this without a
/// `&mut Child`, and the supervisor is holding that — see [`Sessions::stop`]
/// for why signalling by pid is sound in that one position and nowhere else.
#[cfg(unix)]
fn kill_pid(pid: u32) -> AppResult<()> {
    // SAFETY: `kill` with a valid signal number is defined for any pid; a pid
    // that no longer exists returns ESRCH rather than doing something else.
    let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };

    if rc == 0 {
        return Ok(());
    }

    let err = std::io::Error::last_os_error();

    // ESRCH is "it already exited", which is the outcome the caller wanted.
    if err.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }

    Err(AppError::internal(format!(
        "could not stop the game: {err}"
    )))
}

#[cfg(windows)]
fn kill_pid(pid: u32) -> AppResult<()> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    // SAFETY: `OpenProcess` returns null on failure, which is checked; the
    // handle is closed on both paths below.
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);

        if handle.is_null() {
            // Already gone. The caller wanted it stopped and it is.
            return Ok(());
        }

        let ok = TerminateProcess(handle, 1);

        CloseHandle(handle);

        if ok == 0 {
            return Err(AppError::internal(
                "could not stop the game: the system refused to terminate it",
            ));
        }
    }

    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn kill_pid(_pid: u32) -> AppResult<()> {
    Err(AppError::invalid(
        "Stopping a running game is not supported on this platform.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A registry with a log directory of its own.
    ///
    /// Per-test rather than per-process, and that is not tidiness: session ids
    /// restart at 1 for every registry, so two registries sharing a directory
    /// write to the same `session-1.log` — and a capture thread outlives the
    /// exit it was spawned for, so one test's output lands in another's file.
    /// That produced exactly one interleaved line and read as a bug in the
    /// capture rather than in the fixture.
    fn registry() -> Arc<Sessions> {
        static NEXT: AtomicU64 = AtomicU64::new(0);

        let dir = std::env::temp_dir().join(format!(
            "tmc-session-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        Arc::new(Sessions::new(dir))
    }

    #[test]
    fn a_handoff_is_recorded_and_immediately_closed() {
        let sessions = registry();

        let session = sessions.record_handoff(SessionSpec {
            app_id: Some(7),
            label: "Rust".into(),
            ..Default::default()
        });

        assert!(!session.running());
        assert!(sessions.running().is_empty());
        assert_eq!(sessions.recent().len(), 1);
    }

    /// The whole reason [`SessionKind`] exists.
    ///
    /// A handoff that reported a duration would put fabricated playtime on
    /// somebody's account for every `steam://` launch — the number would be the
    /// time between firing a URI and the app noticing, which describes the app
    /// and not the game.
    #[test]
    fn a_handoff_never_reports_playtime() {
        let sessions = registry();

        let session = sessions.record_handoff(SessionSpec {
            label: "Anything".into(),
            ..Default::default()
        });

        assert_eq!(session.seconds(session.started_ms + 3_600_000), None);
    }

    #[test]
    fn a_web_session_runs_until_it_is_ended() {
        let sessions = registry();

        let opened = sessions.open_web(SessionSpec {
            app_id: Some(3),
            label: "A browser game".into(),
            ..Default::default()
        });

        assert!(opened.running());
        assert!(sessions.running_for_app(3));

        let closed = sessions.end(opened.id, true).expect("ended");

        assert!(!closed.running());
        assert!(closed.stopped_by_user);
        assert!(!sessions.running_for_app(3));
    }

    /// Ending twice must not produce two history rows.
    ///
    /// The real case is a stop racing a window-close: both call `end`, and a
    /// second end record would double-count the session's playtime against the
    /// account.
    #[test]
    fn ending_twice_is_idempotent() {
        let sessions = registry();

        let opened = sessions.open_web(SessionSpec {
            label: "Twice".into(),
            ..Default::default()
        });

        assert!(sessions.end(opened.id, false).is_some());
        assert!(sessions.end(opened.id, false).is_none());
        assert_eq!(sessions.recent().len(), 1);
    }

    #[test]
    fn a_spawned_process_is_supervised_to_completion() {
        let sessions = registry();

        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");

            c.args(["/C", "echo hello"]);
            c
        } else {
            let mut c = Command::new("sh");

            c.args(["-c", "echo hello"]);
            c
        };

        command.env("TMC_TEST", "1");

        let session = sessions
            .spawn(
                command,
                SessionSpec {
                    app_id: Some(11),
                    label: "Echo".into(),
                    ..Default::default()
                },
            )
            .expect("spawn");

        assert!(session.running());
        assert!(session.pid.is_some());

        // The supervisor closes it on its own thread; wait for that rather than
        // sleeping a fixed time, which is how this test would flake on a loaded
        // CI runner.
        for _ in 0..200 {
            if !sessions.running_for_app(11) {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert!(!sessions.running_for_app(11), "session never closed");

        let finished = sessions
            .recent()
            .into_iter()
            .find(|s| s.id == session.id)
            .expect("in history");

        assert_eq!(finished.exit_code, Some(0));
        assert!(finished.seconds(now_ms()).is_some());
    }

    #[test]
    fn a_spawned_process_has_its_output_captured() {
        let sessions = registry();

        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");

            c.args(["/C", "echo marker-line"]);
            c
        } else {
            let mut c = Command::new("sh");

            c.args(["-c", "echo marker-line"]);
            c
        };

        command.env("TMC_TEST", "1");

        let session = sessions
            .spawn(
                command,
                SessionSpec {
                    label: "Capture".into(),
                    ..Default::default()
                },
            )
            .expect("spawn");

        for _ in 0..200 {
            if sessions.recent().iter().any(|s| s.id == session.id) {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        // The capture threads are independent of the wait, so the last line can
        // land just after the exit is observed.
        let mut tail = String::new();

        for _ in 0..80 {
            tail = sessions.log_tail(session.id, 50).expect("tail");

            if tail.contains("marker-line") {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert!(tail.contains("marker-line"), "captured: {tail:?}");
    }

    #[test]
    fn the_recent_list_is_bounded() {
        let sessions = registry();

        for i in 0..(MAX_RECENT + 25) {
            let opened = sessions.open_web(SessionSpec {
                label: format!("s{i}"),
                ..Default::default()
            });

            sessions.end(opened.id, false);
        }

        assert_eq!(sessions.recent().len(), MAX_RECENT);
        // Newest first.
        assert_eq!(
            sessions.recent().first().map(|s| s.label.clone()),
            Some(format!("s{}", MAX_RECENT + 24))
        );
    }

    /// Pressing Stop is not a crash, and the history has to be able to say so.
    ///
    /// SIGKILL leaves no exit code, so a stopped session and a session that
    /// died on its own are indistinguishable by `exit_code` alone — which made
    /// every deliberate Stop render under a red warning triangle.
    #[test]
    fn a_stopped_session_is_recorded_as_stopped_by_the_user() {
        let sessions = registry();

        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");

            c.args(["/C", "ping -n 60 127.0.0.1 > nul"]);
            c
        } else {
            let mut c = Command::new("sh");

            c.args(["-c", "sleep 60"]);
            c
        };

        command.env("TMC_TEST", "1");

        let session = sessions
            .spawn(
                command,
                SessionSpec {
                    app_id: Some(77),
                    label: "Stoppable".into(),
                    ..Default::default()
                },
            )
            .expect("spawn");

        sessions.stop(session.id).expect("stop");

        for _ in 0..200 {
            if !sessions.running_for_app(77) {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        let finished = sessions
            .recent()
            .into_iter()
            .find(|s| s.id == session.id)
            .expect("in history");

        assert!(
            finished.stopped_by_user,
            "a deliberate stop was recorded as a crash"
        );
    }

    #[test]
    fn stopping_a_long_running_process_closes_its_session() {
        let sessions = registry();

        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");

            c.args(["/C", "ping -n 60 127.0.0.1 > nul"]);
            c
        } else {
            let mut c = Command::new("sh");

            c.args(["-c", "sleep 60"]);
            c
        };

        command.env("TMC_TEST", "1");

        let session = sessions
            .spawn(
                command,
                SessionSpec {
                    sandbox_id: Some(4),
                    label: "Sleeper".into(),
                    ..Default::default()
                },
            )
            .expect("spawn");

        assert!(sessions.running_for_sandbox(4));

        sessions.stop(session.id).expect("stop");

        for _ in 0..200 {
            if !sessions.running_for_sandbox(4) {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        assert!(!sessions.running_for_sandbox(4), "stop did not take effect");
    }
}
