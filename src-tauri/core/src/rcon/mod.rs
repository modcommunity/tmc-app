//! **RCON** — running commands on a game server from the app.
//!
//! The one place this app deliberately reaches an address the user typed rather
//! than one the API handed it, and the one place it holds a third-party
//! credential. Both are worth stating plainly.
//!
//! WHY THE SSRF GUARD IS NOT APPLIED HERE
//! -------------------------------------
//! Everywhere else, an address goes through [`crate::net::addr::resolve_public`],
//! which refuses anything private — because those addresses arrive as DATA, from
//! the API or from a plugin's template, and probing a user's LAN on a stranger's
//! instruction is the SSRF shape.
//!
//! RCON is the opposite case. `192.168.1.10` and `127.0.0.1` are the *normal*
//! answers: the whole feature is "administer my server", and most people's
//! server is on their own network or their own machine. A guard that refused
//! them would refuse the feature.
//!
//! So the guard is off, and these are what bound it instead:
//!
//!   * **Every connection is audited at Security level**, so it is recorded
//!     even with logging off.
//!   * **Attempts are rate-limited per host**, so the surface cannot be used as
//!     a fast scanner even by something that got script execution in the
//!     webview.
//!   * **Sessions are capped**, so it cannot be used to exhaust sockets.
//!
//! It remains a widening, it is listed as one in `CLAUDE.md`, and it is the
//! honest cost of the feature rather than something the code pretends away.
//!
//! WHERE THE PASSWORD LIVES
//! ------------------------
//! Encrypted on this device and **never sent to the website**. A server
//! password is not ours to hold: it is not derived from a TMC account, losing
//! it costs somebody their server rather than their profile, and there is no
//! feature on the site that needs it. See [`crate::crypto`] for what the
//! encryption does and does not buy.

pub mod frostbite;
pub mod source;
pub mod store;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::audit;
use crate::error::{AppError, AppResult};
use crate::logging::Audit;

/// Cap on live sessions. More than this is somebody doing something other than
/// administering their servers.
pub const MAX_SESSIONS: usize = 8;

/// A session with nothing sent on it for this long is closed. Most servers drop
/// an idle RCON connection themselves, and holding a dead socket means the next
/// command fails instead of reconnecting.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// The shortest gap between connection attempts to one host.
const CONNECT_COOLDOWN: Duration = Duration::from_millis(750);

/// Default per-command timeout.
pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;

/// Which dialect a server speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RconProtocol {
    /// Valve's, which is also Minecraft's, Rust's, ARK's, Squad's, Palworld's
    /// and most of the rest. The default because it is nearly all of them.
    #[default]
    Source,
    /// Battlefield 3 / 4 / Hardline / Bad Company 2.
    Frostbite,
}

impl RconProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Frostbite => "frostbite",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "source" | "valve" | "minecraft" => Some(Self::Source),
            "frostbite" | "battlefield" => Some(Self::Frostbite),
            _ => None,
        }
    }
}

/// Where to connect, and how.
#[derive(Debug, Clone)]
pub struct RconTarget {
    pub host: String,
    pub port: u16,
    pub protocol: RconProtocol,
    pub timeout: Duration,
}

impl RconTarget {
    fn key(&self) -> String {
        format!("{}:{}:{}", self.host, self.port, self.protocol.as_str())
    }
}

/// What one command produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RconReply {
    pub command: String,
    pub output: String,
    /// Milliseconds from send to last byte.
    pub took_ms: u64,
}

/// An open, authenticated connection.
enum Session {
    Source(Box<source::SourceRcon>),
    Frostbite(Box<frostbite::FrostbiteRcon>),
}

impl Session {
    async fn exec(&mut self, command: &str) -> AppResult<String> {
        match self {
            Self::Source(rcon) => rcon.exec(command).await,
            Self::Frostbite(rcon) => rcon.exec(command).await,
        }
    }
}

struct Live {
    session: Session,
    last_used: Instant,
}

/// Open connections, one per target.
///
/// A pool rather than connect-per-command, because an RCON console is a
/// conversation: reconnecting and re-authenticating for every line adds two
/// round trips to each one and, on servers that rate-limit authentication,
/// eventually gets the app locked out for using it as intended.
#[derive(Clone)]
pub struct RconPool {
    inner: Arc<Inner>,
}

struct Inner {
    sessions: Mutex<HashMap<String, Live>>,
    /// Last connection attempt per host, for the cooldown.
    attempts: Mutex<HashMap<String, Instant>>,
    audit: Arc<Audit>,
}

impl RconPool {
    pub fn new(audit: Arc<Audit>) -> Self {
        Self {
            inner: Arc::new(Inner {
                sessions: Mutex::new(HashMap::new()),
                attempts: Mutex::new(HashMap::new()),
                audit,
            }),
        }
    }

    /// Run one command, connecting first if there is no live session.
    pub async fn exec(
        &self,
        target: &RconTarget,
        password: &str,
        command: &str,
    ) -> AppResult<RconReply> {
        let key = target.key();
        let started = Instant::now();

        self.sweep_idle().await;

        // A first attempt on an existing session. If it fails, the session is
        // dropped and the command is tried once more on a fresh one — a server
        // that closed an idle connection is the ordinary case, and making the
        // user click again for it is noise.
        if let Some(output) = self.try_existing(&key, command).await? {
            return Ok(RconReply {
                command: command.to_string(),
                output,
                took_ms: started.elapsed().as_millis() as u64,
            });
        }

        let session = self.connect(target, password).await?;

        {
            let mut sessions = self.inner.sessions.lock().await;

            if sessions.len() >= MAX_SESSIONS {
                // Evict the least recently used rather than refusing: somebody
                // with nine servers is doing nothing wrong.
                if let Some(oldest) = sessions
                    .iter()
                    .min_by_key(|(_, live)| live.last_used)
                    .map(|(k, _)| k.clone())
                {
                    sessions.remove(&oldest);
                }
            }

            sessions.insert(
                key.clone(),
                Live {
                    session,
                    last_used: Instant::now(),
                },
            );
        }

        let output = self
            .try_existing(&key, command)
            .await?
            .ok_or_else(|| AppError::Network("The connection closed immediately.".into()))?;

        Ok(RconReply {
            command: command.to_string(),
            output,
            took_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// Run on the pooled session, or `None` if there is not a usable one.
    async fn try_existing(&self, key: &str, command: &str) -> AppResult<Option<String>> {
        let mut sessions = self.inner.sessions.lock().await;

        let Some(live) = sessions.get_mut(key) else {
            return Ok(None);
        };

        match live.session.exec(command).await {
            Ok(output) => {
                live.last_used = Instant::now();

                Ok(Some(output))
            }
            Err(AppError::Network(_)) => {
                // The socket is gone. Drop it and let the caller reconnect.
                sessions.remove(key);

                Ok(None)
            }
            /*
             * Anything else — a refused command, a too-long command — is the
             * server's answer and belongs to the user, not a reason to
             * reconnect and try again.
             */
            Err(other) => Err(other),
        }
    }

    async fn connect(&self, target: &RconTarget, password: &str) -> AppResult<Session> {
        validate(target)?;
        self.cooldown(&target.host).await?;

        /*
         * Resolved WITHOUT the public-address guard — see the module header.
         * Still resolved once and connected to the result, so a hostname that
         * answers differently on a second lookup cannot redirect the session.
         */
        let addr = resolve(&target.host, target.port).await?;

        audit!(
            self.inner.audit,
            Security,
            Network,
            "rcon.connect",
            format!(
                "{}:{} ({})",
                target.host,
                target.port,
                target.protocol.as_str()
            )
        );

        let session = match target.protocol {
            RconProtocol::Source => Session::Source(Box::new(
                source::SourceRcon::connect(addr, password, target.timeout).await?,
            )),
            RconProtocol::Frostbite => Session::Frostbite(Box::new(
                frostbite::FrostbiteRcon::connect(addr, password, target.timeout).await?,
            )),
        };

        Ok(session)
    }

    /// Close one session.
    pub async fn disconnect(&self, target: &RconTarget) {
        self.inner.sessions.lock().await.remove(&target.key());
    }

    /// Close everything. Called on sign-out and on shutdown.
    pub async fn disconnect_all(&self) {
        self.inner.sessions.lock().await.clear();
    }

    /// Is there a live session for this target?
    pub async fn is_connected(&self, target: &RconTarget) -> bool {
        self.inner.sessions.lock().await.contains_key(&target.key())
    }

    async fn sweep_idle(&self) {
        let mut sessions = self.inner.sessions.lock().await;

        sessions.retain(|_, live| live.last_used.elapsed() < IDLE_TIMEOUT);
    }

    /// Refuse a second attempt at the same host too soon after the last.
    ///
    /// Not a security boundary on its own — it is what stops this surface being
    /// a fast port scanner, given that it deliberately reaches private
    /// addresses.
    async fn cooldown(&self, host: &str) -> AppResult<()> {
        let mut attempts = self.inner.attempts.lock().await;

        // Bounded, so a script trying a thousand hosts cannot grow this map.
        if attempts.len() > 256 {
            attempts.retain(|_, at| at.elapsed() < CONNECT_COOLDOWN * 4);
        }

        if let Some(last) = attempts.get(host) {
            if last.elapsed() < CONNECT_COOLDOWN {
                return Err(AppError::invalid(
                    "Give the server a moment before connecting again.",
                ));
            }
        }

        attempts.insert(host.to_string(), Instant::now());

        Ok(())
    }
}

/// Run a command against a SAVED server.
///
/// **This is the only way anything above this crate reaches a stored
/// password.** The decryption, the connection and the command all happen
/// inside one call that takes an id and returns output, so there is no shape
/// the Tauri command layer could take that would hand a password to the
/// webview — the function that could is `pub(crate)` and this is its only
/// caller.
///
/// The command and its output are appended to the server's history whichever
/// way it went. A console that records only what succeeded is missing exactly
/// the half somebody comes back to read.
pub async fn exec_saved(
    db: &crate::library::LibraryDb,
    cipher: &crate::crypto::LocalCipher,
    pool: &RconPool,
    server_id: i64,
    command: &str,
    timeout: Duration,
) -> AppResult<RconReply> {
    let server = db
        .rcon_get(server_id)?
        .ok_or_else(|| AppError::invalid("That server is not saved."))?;

    let password = db.rcon_secret(cipher, server_id)?;

    let target = RconTarget {
        host: server.host.clone(),
        port: server.port,
        protocol: server.protocol,
        timeout,
    };

    let result = pool.exec(&target, &password, command).await;

    match &result {
        Ok(reply) => {
            let _ = db.rcon_log(server_id, command, &reply.output, true);
            let _ = db.rcon_touch(server_id);
        }
        Err(err) => {
            let _ = db.rcon_log(server_id, command, &err.to_string(), false);
        }
    }

    result
}

/// Open a connection to a saved server without running anything.
///
/// What the console's "connect" button does, so a wrong password is reported
/// before somebody types a command into a pane that looks live.
pub async fn connect_saved(
    db: &crate::library::LibraryDb,
    cipher: &crate::crypto::LocalCipher,
    pool: &RconPool,
    server_id: i64,
    timeout: Duration,
) -> AppResult<()> {
    /*
     * A no-op command rather than a bare connect: authentication is the thing
     * being tested, and on both protocols it happens during the handshake, so
     * the cheapest command that proves the session works is the honest test.
     */
    let probe = match db
        .rcon_get(server_id)?
        .ok_or_else(|| AppError::invalid("That server is not saved."))?
        .protocol
    {
        RconProtocol::Source => "",
        RconProtocol::Frostbite => "version",
    };

    exec_saved(db, cipher, pool, server_id, probe, timeout).await?;

    Ok(())
}

/// Resolve, once, without the public-address requirement.
async fn resolve(host: &str, port: u16) -> AppResult<SocketAddr> {
    tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| AppError::Network(format!("Could not find {host}.")))?
        .next()
        .ok_or_else(|| AppError::Network(format!("{host} did not resolve to an address.")))
}

fn validate(target: &RconTarget) -> AppResult<()> {
    if target.host.trim().is_empty() || target.host.len() > 253 {
        return Err(AppError::invalid("Enter the server's address."));
    }

    // A host with a scheme or a path in it is a URL somebody pasted, and
    // `lookup_host` would fail with something unhelpful.
    if target.host.contains("://") || target.host.contains('/') {
        return Err(AppError::invalid(
            "Enter just the address — no http:// and no path.",
        ));
    }

    if target.port == 0 {
        return Err(AppError::invalid("Enter the RCON port."));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(host: &str, port: u16) -> RconTarget {
        RconTarget {
            host: host.into(),
            port,
            protocol: RconProtocol::Source,
            timeout: Duration::from_millis(500),
        }
    }

    #[test]
    fn protocols_round_trip_through_their_wire_names() {
        for protocol in [RconProtocol::Source, RconProtocol::Frostbite] {
            assert_eq!(RconProtocol::parse(protocol.as_str()), Some(protocol));
        }

        // Minecraft speaks Valve's protocol; the alias exists so a stored
        // "minecraft" keeps working rather than falling back to a default.
        assert_eq!(RconProtocol::parse("minecraft"), Some(RconProtocol::Source));
        assert_eq!(RconProtocol::parse("nonsense"), None);
    }

    #[test]
    fn a_pasted_url_is_refused_with_a_useful_message() {
        for host in ["http://play.example.com", "play.example.com/rcon", ""] {
            let err = validate(&target(host, 27015)).expect_err("must refuse");

            assert!(!err.to_string().is_empty());
        }

        assert!(validate(&target("play.example.com", 0)).is_err());
        assert!(validate(&target("play.example.com", 27015)).is_ok());
        // The point of the module: a LAN address is the normal case here.
        assert!(validate(&target("192.168.1.10", 27015)).is_ok());
        assert!(validate(&target("127.0.0.1", 27015)).is_ok());
    }

    #[tokio::test]
    async fn connecting_twice_in_a_row_is_slowed_down() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let pool = RconPool::new(Arc::new(Audit::new(tmp.path().join("a.jsonl"))));

        pool.cooldown("192.168.1.10").await.expect("first is fine");

        assert!(
            pool.cooldown("192.168.1.10").await.is_err(),
            "a second immediate attempt at the same host must be refused"
        );

        // A different host is unaffected — the limit is per host, so somebody
        // with several servers is not slowed down.
        assert!(pool.cooldown("192.168.1.11").await.is_ok());
    }

    #[tokio::test]
    async fn a_target_with_no_session_reports_itself_disconnected() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let pool = RconPool::new(Arc::new(Audit::new(tmp.path().join("a.jsonl"))));

        assert!(!pool.is_connected(&target("play.example.com", 27015)).await);
    }

    #[tokio::test]
    async fn a_connection_to_a_dead_port_fails_rather_than_hanging() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let pool = RconPool::new(Arc::new(Audit::new(tmp.path().join("a.jsonl"))));

        // Port 1 on loopback: nothing listens, and the refusal is immediate.
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            pool.exec(&target("127.0.0.1", 1), "pw", "status"),
        )
        .await
        .expect("must not hang");

        assert!(result.is_err());
    }
}
