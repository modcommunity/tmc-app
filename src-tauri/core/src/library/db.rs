//! The device-local library database.
//!
//! WHY SQLITE AND NOT A JSON FILE
//! -----------------------------
//! Everything else this app persists (settings, the plugin registry) is a small
//! JSON file rewritten whole, and that is right for those: they are read at
//! launch, written on a click, and never contended. The library is not like
//! that. The sync loop writes it on a timer while the UI reads it on every
//! render and the installer writes single rows mid-install — and a whole-file
//! rewrite under that pattern loses one writer to another, silently, exactly
//! when a user is watching a progress bar.
//!
//! WHAT IS IN HERE AND WHAT IS NOT
//! -------------------------------
//! Only facts about THIS machine:
//!
//!   * which subscribed items are materialised, at which release, into which
//!     install directory, and which files were written;
//!   * the sync watermark;
//!   * the last error, per item, so a failure is visible rather than a retry
//!     loop nobody can see.
//!
//! The subscription itself is the SERVER's fact and is mirrored here only so
//! the app can render its library offline and diff against it. Anything a user
//! changes here is written to the API first and mirrored second — the local row
//! is never the authority.
//!
//! CONCURRENCY
//! -----------
//! One connection behind a `Mutex`, in WAL mode. The alternative — a pool — buys
//! nothing: every write is a handful of rows and the readers are a UI that
//! refreshes on a timer. WAL is what stops a read blocking behind the sync
//! loop's write, which is the only contention that actually happens.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// Schema version. Bumped whenever `migrate` gains a step.
const SCHEMA_VERSION: i64 = 9;

/// One subscribed item as this device knows it.
///
/// The server's fields and the device's, side by side, because every screen
/// that shows one wants the other — "Cool Mod, v1.4 available, v1.3 installed"
/// is one row, and splitting it would mean a join in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    // ---------------------------------------------------------- from the API
    pub id: String,
    pub kind: String,
    pub item_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub web_url: String,
    pub app_id: Option<i64>,
    pub app_name: Option<String>,
    /// The app's URL slug — what selects `plugins/app/<slug>/…`.
    pub app_slug: Option<String>,
    pub auto_update: bool,
    pub notify_updates: bool,
    pub paused: bool,
    pub via_collection_id: Option<i64>,
    pub installable: bool,
    /// Newest release the server knows about.
    pub latest_release_id: Option<i64>,
    pub latest_version: Option<String>,
    pub file_url: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub file_sha256: Option<String>,
    pub updated_at: String,

    // ------------------------------------------------------- device-local
    /// Which release is currently on disk, or `None`.
    pub installed_release_id: Option<i64>,
    pub installed_version: Option<String>,
    /// Which sandbox (cloud `install`) it was materialised into. `None` = the default.
    pub installed_install_id: Option<i64>,
    pub installed_at: Option<String>,
    /// Paths the installer wrote, so the uninstall knows what to remove.
    pub installed_files: Vec<String>,
    /// `idle`, `queued`, `installing`, `installed`, `failed`, `removing`.
    pub state: String,
    pub last_error: Option<String>,
}

impl LibraryEntry {
    /// Is there a newer release than the one on disk?
    ///
    /// Compares release IDS, not version strings. Version strings are authored
    /// by hand and are not ordered in any way a machine can rely on — `1.10`
    /// sorts before `1.9` lexically and after it numerically, and plenty of
    /// items version by date or by commit hash. The release id is monotonic
    /// because the database mints it.
    pub fn update_available(&self) -> bool {
        match (self.latest_release_id, self.installed_release_id) {
            (Some(latest), Some(installed)) => latest != installed,
            (Some(_), None) => false,
            _ => false,
        }
    }

    /// Should this device be installing it at all?
    ///
    /// A COLLECTION is never installed directly, and that is not an omission.
    /// Subscribing to one fans out to a real subscription row per member (see
    /// the server's `SubscribeCollectionItems`), so the members are what the
    /// installer acts on. The collection row is a container: it names the group
    /// in the library, and removing it removes the rows it created.
    ///
    /// Without this the installer would look for a `manage_collection` rule for
    /// a kind that has no app slug at all, and every collection in a user's
    /// library would sit permanently at "No install rule for this game" — an
    /// error about something that was never going to happen.
    pub fn should_install(&self) -> bool {
        self.installable && !self.paused && !self.is_container() && self.file_url.is_some()
    }

    /// Is this a grouping rather than something with files?
    pub fn is_container(&self) -> bool {
        self.kind == "collection"
    }
}

pub struct LibraryDb {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl LibraryDb {
    /// Open (creating if needed) and migrate.
    pub fn open(path: PathBuf) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn =
            Connection::open(&path).map_err(|e| AppError::internal(format!("library db: {e}")))?;

        /*
         * WAL so a read never blocks behind the sync loop's write. `NORMAL`
         * synchronous rather than `FULL`: this database is a CACHE of server
         * state plus a record of what is on disk, and both are reconstructible —
         * losing the last few milliseconds to a power cut costs one re-sync,
         * while `FULL` costs an fsync per install step.
         */
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| AppError::internal(format!("library db pragma: {e}")))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| AppError::internal(format!("library db pragma: {e}")))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| AppError::internal(format!("library db pragma: {e}")))?;
        // A busy database should wait, not fail: the only writers are our own
        // sync loop and our own installer, and they are both short.
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| AppError::internal(format!("library db busy: {e}")))?;

        let db = Self {
            conn: Mutex::new(conn),
            path,
        };

        db.migrate()?;

        Ok(db)
    }

    /// In-memory, for tests.
    pub fn open_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| AppError::internal(format!("library db: {e}")))?;

        let db = Self {
            conn: Mutex::new(conn),
            path: PathBuf::from(":memory:"),
        };

        db.migrate()?;

        Ok(db)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn with<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> AppResult<T> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| AppError::internal("library db lock poisoned"))?;

        f(&conn).map_err(|e| AppError::internal(format!("library db: {e}")))
    }

    /// Create or upgrade the schema.
    ///
    /// `user_version` rather than a table: it is a single integer SQLite already
    /// maintains, it costs no query to read, and it cannot itself be missing —
    /// which a migrations table can be, on exactly the databases that most need
    /// migrating.
    ///
    /// **Steps, not one batch.** Each step takes the database from `n-1` to `n`
    /// and is applied only if it has not been. Re-running the whole batch
    /// happens to be safe today because every statement is `IF NOT EXISTS`, and
    /// it stops being safe the first time a step needs an `ALTER TABLE` — which
    /// is the release where somebody discovers it, on a user's data.
    fn migrate(&self) -> AppResult<()> {
        self.with(|conn| {
            let mut version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

            while version < SCHEMA_VERSION {
                let next = version + 1;

                match next {
                    1 => conn.execute_batch(SCHEMA_V1)?,
                    2 => conn.execute_batch(SCHEMA_V2)?,
                    3 => conn.execute_batch(SCHEMA_V3)?,
                    4 => conn.execute_batch(SCHEMA_V4)?,
                    5 => conn.execute_batch(SCHEMA_V5)?,
                    6 => conn.execute_batch(SCHEMA_V6)?,
                    7 => conn.execute_batch(SCHEMA_V7)?,
                    8 => conn.execute_batch(SCHEMA_V8)?,
                    9 => conn.execute_batch(SCHEMA_V9)?,
                    _ => break,
                }

                conn.pragma_update(None, "user_version", next)?;

                version = next;
            }

            Ok(())
        })
    }
}

const SCHEMA_V1: &str = r#"
                CREATE TABLE IF NOT EXISTS subscription (
                    id                  TEXT PRIMARY KEY,
                    kind                TEXT NOT NULL,
                    item_id             INTEGER NOT NULL,
                    name                TEXT NOT NULL,
                    description         TEXT,
                    image               TEXT,
                    web_url             TEXT NOT NULL,
                    app_id              INTEGER,
                    app_name            TEXT,
                    app_slug            TEXT,
                    auto_update         INTEGER NOT NULL DEFAULT 1,
                    notify_updates      INTEGER NOT NULL DEFAULT 1,
                    paused              INTEGER NOT NULL DEFAULT 0,
                    via_collection_id   INTEGER,
                    installable         INTEGER NOT NULL DEFAULT 1,
                    latest_release_id   INTEGER,
                    latest_version      TEXT,
                    file_url            TEXT,
                    file_name           TEXT,
                    file_size           INTEGER,
                    file_sha256         TEXT,
                    updated_at          TEXT NOT NULL,

                    installed_release_id INTEGER,
                    installed_version    TEXT,
                    installed_install_id INTEGER,
                    installed_at         TEXT,
                    installed_files      TEXT NOT NULL DEFAULT '[]',
                    state                TEXT NOT NULL DEFAULT 'idle',
                    last_error           TEXT
                );

                CREATE INDEX IF NOT EXISTS subscription_kind_idx
                    ON subscription (kind, item_id);
                CREATE INDEX IF NOT EXISTS subscription_state_idx
                    ON subscription (state);
                CREATE INDEX IF NOT EXISTS subscription_app_idx
                    ON subscription (app_slug);

                /* The sync watermark and anything else that is one value. */
                CREATE TABLE IF NOT EXISTS meta (
                    key   TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                /*
                 * Installs mirrored from the cloud, plus the one fact that is
                 * NOT in the cloud: which directory on THIS machine it was
                 * materialised into. That is why the mirror exists at all —
                 * without it the app cannot answer "where did this go?" while
                 * offline.
                 */
                CREATE TABLE IF NOT EXISTS install (
                    id            INTEGER PRIMARY KEY,
                    app_id        INTEGER NOT NULL,
                    app_slug      TEXT,
                    name          TEXT NOT NULL,
                    is_default    INTEGER NOT NULL DEFAULT 0,
                    game_version  TEXT,
                    loader        TEXT,
                    local_dir     TEXT,
                    updated_at    TEXT NOT NULL,
                    payload       TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS install_app_idx ON install (app_id);
"#;

/// Mods that are on this machine without an account behind them.
///
/// Its own table rather than rows in `subscription`, and the reason is the same
/// one that keeps `sandbox` out of `install`: `subscription` is a MIRROR of the
/// account, rewritten by every sync, and a full sync deletes anything the
/// server did not send. A dropped jar would not survive its first sync — which
/// is the one behaviour an imported mod must never have.
///
/// It is also why there is no `latest_release_id` here. A local mod has no
/// release history, no checksum from anybody and no URL to re-fetch; a column
/// for one would be a column that is always null and an update path that is
/// always a lie. See `crate::local`.
///
/// `source_*` is the one link back to the site, set only when an archive's own
/// `tmc.json` named an item AND named this build's API base. It drives an
/// offer — "subscribe to this instead" — and nothing automatic.
const SCHEMA_V8: &str = r#"
                CREATE TABLE IF NOT EXISTS local_mod (
                    id           INTEGER PRIMARY KEY AUTOINCREMENT,
                    name         TEXT NOT NULL,
                    app_id       INTEGER,
                    app_slug     TEXT,
                    version      TEXT,
                    author       TEXT,
                    notes        TEXT,
                    /* dropped | folder | adopted | manager */
                    origin       TEXT NOT NULL,
                    origin_label TEXT,
                    /* Where its files sit under the game folder. May be ''. */
                    rel_path     TEXT NOT NULL DEFAULT '',

                    source_kind    TEXT,
                    source_item    INTEGER,
                    source_release INTEGER,
                    source_url     TEXT,

                    files        INTEGER NOT NULL DEFAULT 0,
                    bytes        INTEGER NOT NULL DEFAULT 0,
                    added_at     TEXT NOT NULL,
                    updated_at   TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS local_mod_app_idx ON local_mod (app_id);
                CREATE INDEX IF NOT EXISTS local_mod_origin_idx ON local_mod (origin);
"#;

/// Sandboxes, their mods, and what the last deploy of each put on disk.
///
/// Separate from `install` rather than columns added to it, and the difference
/// matters: `install` is a MIRROR of a cloud row and is rewritten wholesale by
/// every sync, while `sandbox` is the device's own record and includes
/// sandboxes the cloud has never heard of (see `cloud_sync`). Merging the two
/// would mean a sync deleting a local-only sandbox because the server did not
/// list it.
/// Whether a sandbox keeps its own mods up to date.
///
/// The first migration that is an `ALTER TABLE` rather than a `CREATE ... IF
/// NOT EXISTS`, which is exactly the case the stepwise migration runner exists
/// for: re-running this batch on a database that already has the column is an
/// error, not a no-op.
const SCHEMA_V6: &str = r#"
                ALTER TABLE sandbox
                    ADD COLUMN auto_update INTEGER NOT NULL DEFAULT 1;
"#;

/// Dependency edges, cached per item.
///
/// WHY THEY ARE CACHED AT ALL
/// --------------------------
/// Checking a sandbox for missing requirements and internal conflicts is a
/// question about EVERY item in it at once. Asking the API per item would be
/// forty requests before a deploy, on a screen somebody is waiting on, for data
/// that changes about as often as a mod is re-released.
///
/// So an item's edges are fetched once — when it is added to a sandbox — and
/// re-fetched when it is staged. Between those the check is a local join and
/// costs nothing.
///
/// STALENESS IS ACCEPTABLE HERE AND IS NOT ELSEWHERE
/// ------------------------------------------------
/// These rows drive a WARNING, never a refusal. An author who adds a
/// requirement today and a user who deploys before their next sync sees no
/// warning, which is exactly what they saw yesterday. Nothing is installed or
/// removed on the strength of this table.
const SCHEMA_V5: &str = r#"
                CREATE TABLE IF NOT EXISTS item_dependency (
                    /* The item that HAS the dependency. */
                    kind      TEXT NOT NULL,
                    item_id   INTEGER NOT NULL,

                    /* The other end. */
                    rel_kind  TEXT NOT NULL,
                    rel_id    INTEGER NOT NULL,

                    /* Required | Optional | Recommended | Conflict */
                    relation  TEXT NOT NULL,
                    name      TEXT NOT NULL,
                    icon      TEXT,
                    note      TEXT,
                    fetched_at TEXT NOT NULL,

                    PRIMARY KEY (kind, item_id, rel_kind, rel_id)
                );

                CREATE INDEX IF NOT EXISTS item_dependency_rel_idx
                    ON item_dependency (rel_kind, rel_id);
"#;

/// Saved RCON servers and their console history.
///
/// The `password` column holds CIPHERTEXT, never a password — see
/// `crate::rcon::store`. Nothing in here is ever sent to the website.
const SCHEMA_V4: &str = r#"
                CREATE TABLE IF NOT EXISTS rcon_server (
                    id           INTEGER PRIMARY KEY AUTOINCREMENT,
                    name         TEXT NOT NULL,
                    host         TEXT NOT NULL,
                    port         INTEGER NOT NULL,
                    /* source | frostbite */
                    protocol     TEXT NOT NULL DEFAULT 'source',
                    /* base64(nonce || XChaCha20-Poly1305 ciphertext), or NULL. */
                    password     TEXT,
                    /* The TMC server row this was created from, when it was. */
                    server_id    INTEGER,
                    app_id       INTEGER,
                    last_used_at TEXT,
                    created_at   TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS rcon_server_ref_idx ON rcon_server (server_id);

                CREATE TABLE IF NOT EXISTS rcon_history (
                    id        INTEGER PRIMARY KEY AUTOINCREMENT,
                    server_id INTEGER NOT NULL
                              REFERENCES rcon_server (id) ON DELETE CASCADE,
                    command   TEXT NOT NULL,
                    output    TEXT NOT NULL,
                    ok        INTEGER NOT NULL DEFAULT 1,
                    at        TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS rcon_history_server_idx
                    ON rcon_history (server_id, id DESC);
"#;

/// The download queue, so it survives the app closing.
///
/// Kept in the same database as everything else device-local rather than in its
/// own file: a completed download hands a staged file to a sandbox, and the two
/// facts wanting to be written together is exactly the case one file per
/// subsystem gets wrong.
const SCHEMA_V3: &str = r#"
                CREATE TABLE IF NOT EXISTS download (
                    id         TEXT PRIMARY KEY,
                    url        TEXT NOT NULL,
                    /* Absolute, and already through the jail when it was queued. */
                    dest       TEXT NOT NULL,
                    label      TEXT NOT NULL,
                    sha256     TEXT,
                    total      INTEGER,
                    /* A HINT. The `.part` file's length is the real answer. */
                    done       INTEGER NOT NULL DEFAULT 0,
                    status     TEXT NOT NULL,
                    priority   INTEGER NOT NULL DEFAULT 0,
                    limit_bps  INTEGER,
                    attempts   INTEGER NOT NULL DEFAULT 0,
                    error      TEXT,
                    /* Whatever the caller attached — the sandbox id, the mod key. */
                    meta       TEXT NOT NULL DEFAULT '{}',
                    queued_at  TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS download_status_idx ON download (status);
"#;

const SCHEMA_V2: &str = r#"
                CREATE TABLE IF NOT EXISTS sandbox (
                    id            INTEGER PRIMARY KEY AUTOINCREMENT,
                    /* `AppInstall.id`, or NULL for a sandbox kept off the cloud. */
                    remote_id     INTEGER UNIQUE,
                    app_id        INTEGER NOT NULL,
                    app_slug      TEXT,
                    app_name      TEXT,
                    name          TEXT NOT NULL,
                    description   TEXT,
                    /* client | server | shared */
                    environment   TEXT NOT NULL DEFAULT 'client',
                    /* direct | hardlink | symlink | usvfs */
                    strategy      TEXT NOT NULL DEFAULT 'hardlink',
                    game_version  TEXT,
                    loader        TEXT,
                    preset        TEXT,
                    is_default    INTEGER NOT NULL DEFAULT 0,
                    /* Whether this sandbox's definition is mirrored to the account. */
                    cloud_sync    INTEGER NOT NULL DEFAULT 1,
                    /* The game folder on THIS machine. NULL = the app's configured one. */
                    game_dir      TEXT,
                    options       TEXT NOT NULL DEFAULT '{}',
                    launch_args   TEXT NOT NULL DEFAULT '[]',
                    launch_env    TEXT NOT NULL DEFAULT '{}',
                    deployed_at   TEXT,
                    /* The last deploy's report, for the UI. Advisory. */
                    last_deploy   TEXT,
                    created_at    TEXT NOT NULL,
                    updated_at    TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS sandbox_app_idx ON sandbox (app_id);

                CREATE TABLE IF NOT EXISTS sandbox_mod (
                    sandbox_id  INTEGER NOT NULL
                                REFERENCES sandbox (id) ON DELETE CASCADE,
                    /* `mod:1234` / `asset:9` — what the merge tree and the ledger use. */
                    mod_key     TEXT NOT NULL,
                    kind        TEXT NOT NULL,
                    item_id     INTEGER NOT NULL,
                    name        TEXT NOT NULL,
                    enabled     INTEGER NOT NULL DEFAULT 1,
                    /* Higher wins a contested path. */
                    priority    INTEGER NOT NULL DEFAULT 0,
                    /* Which release is in staging, and where it came from. */
                    release_id  INTEGER,
                    version     TEXT,
                    staged_at   TEXT,
                    last_error  TEXT,

                    PRIMARY KEY (sandbox_id, mod_key)
                );

                CREATE INDEX IF NOT EXISTS sandbox_mod_order_idx
                    ON sandbox_mod (sandbox_id, priority);

                /*
                 * The deployment ledger: every file the last deploy put into
                 * the game folder, and what it displaced to get there.
                 *
                 * The one table that must survive a crash intact, because it is
                 * the only record of which files in somebody's game folder
                 * belong to us. See `deploy::ledger` for why a rescan cannot
                 * replace it.
                 */
                CREATE TABLE IF NOT EXISTS deployment (
                    sandbox_id INTEGER NOT NULL
                               REFERENCES sandbox (id) ON DELETE CASCADE,
                    path       TEXT NOT NULL,
                    kind       TEXT NOT NULL,
                    mod_key    TEXT NOT NULL,
                    source     TEXT NOT NULL,
                    size       INTEGER NOT NULL,
                    mtime_ms   INTEGER NOT NULL,
                    backup     TEXT,

                    PRIMARY KEY (sandbox_id, path)
                );

                CREATE INDEX IF NOT EXISTS deployment_mod_idx
                    ON deployment (sandbox_id, mod_key);
"#;

const SCHEMA_V7: &str = r#"
                /*
                 * Every launch this device has made, and how long it lasted.
                 *
                 * Durable rather than in-memory because the two questions it
                 * answers outlive a process: "how long have I played this" is a
                 * running total, and "what did I play last" is what orders the
                 * Library. `session::Sessions` holds the LIVE half; this is
                 * where a finished one lands.
                 *
                 * NO foreign key to `sandbox`, deliberately. A sandbox is
                 * deleted when somebody is done with a mod list, and cascading
                 * would erase the record of having played it — so `sandbox_id`
                 * is allowed to dangle and every reader treats a missing
                 * sandbox as "a profile that no longer exists" rather than as a
                 * broken row.
                 *
                 * `seconds` is stored rather than derived from the two
                 * timestamps because it is not always their difference: a
                 * handoff launch has no measurable duration at all and stores
                 * 0, and reconstructing that rule at every read is how one
                 * reader eventually gets it wrong. See `session::SessionKind`.
                 */
                CREATE TABLE IF NOT EXISTS game_session (
                    id              INTEGER PRIMARY KEY AUTOINCREMENT,
                    kind            TEXT    NOT NULL,
                    app_id          INTEGER,
                    app_slug        TEXT,
                    label           TEXT    NOT NULL,
                    sandbox_id      INTEGER,
                    install_id      INTEGER,
                    started_ms      INTEGER NOT NULL,
                    ended_ms        INTEGER,
                    seconds         INTEGER NOT NULL DEFAULT 0,
                    exit_code       INTEGER,
                    stopped_by_user INTEGER NOT NULL DEFAULT 0,
                    /* Whether the account has been told about this session. */
                    reported        INTEGER NOT NULL DEFAULT 0
                );

                CREATE INDEX IF NOT EXISTS game_session_app_idx
                    ON game_session (app_id, started_ms DESC);

                CREATE INDEX IF NOT EXISTS game_session_sandbox_idx
                    ON game_session (sandbox_id, started_ms DESC);

                CREATE INDEX IF NOT EXISTS game_session_unreported_idx
                    ON game_session (reported, install_id);
"#;

const SCHEMA_V9: &str = r#"
                /*
                 * A GAME THIS DEVICE INSTALLED FROM TMC ITSELF.
                 *
                 * Its own table rather than a column on `install` or a row in
                 * `subscription`, for the reason `local_mod` has its own:
                 * `subscription` is a MIRROR that the full sync rewrites, and
                 * anything the server did not send is deleted from it. A game
                 * on this disk is a fact about this disk — the account knows
                 * which games exist, never which of them somebody unpacked
                 * onto a laptop — so a sync must not be able to forget it.
                 *
                 * Keyed by app id, because one machine installs one build of a
                 * game. `platform` records WHICH build, so a library moved
                 * between machines (or a laptop that changed architecture
                 * under Rosetta) is visibly wrong rather than quietly broken.
                 */
                CREATE TABLE IF NOT EXISTS native_game (
                    app_id        INTEGER PRIMARY KEY,
                    slug          TEXT,
                    name          TEXT    NOT NULL,
                    platform      TEXT    NOT NULL,
                    version       TEXT    NOT NULL,
                    /* Absolute, and the only thing that knows where it landed. */
                    dir           TEXT    NOT NULL,
                    /* Relative to `dir`. Null only for a single-file build. */
                    entry         TEXT,
                    /* The launch argument template, as a JSON array. */
                    args          TEXT    NOT NULL DEFAULT '[]',
                    size_bytes    INTEGER NOT NULL DEFAULT 0,
                    installed_ms  INTEGER NOT NULL,
                    updated_ms    INTEGER NOT NULL,
                    /* Whether the update pass may replace this without asking. */
                    auto_update   INTEGER NOT NULL DEFAULT 1
                );
"#;

/// One finished launch, as the database holds it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    pub id: i64,
    pub kind: String,
    pub app_id: Option<i64>,
    pub app_slug: Option<String>,
    pub label: String,
    pub sandbox_id: Option<i64>,
    pub install_id: Option<i64>,
    pub started_ms: i64,
    pub ended_ms: Option<i64>,
    /// Measured seconds. Always 0 for a launch whose duration is unknowable.
    pub seconds: i64,
    pub exit_code: Option<i32>,
    pub stopped_by_user: bool,
}

/// Play totals for one game or one sandbox.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayTotals {
    pub seconds: i64,
    pub launches: i64,
    /// When it was last started, or null for something never played here.
    pub last_played_ms: Option<i64>,
}

impl LibraryDb {
    // -------------------------------------------------------------- sessions

    /// Write a finished session.
    ///
    /// Takes the already-computed `seconds` rather than deriving it, for the
    /// reason the schema comment gives: a handoff's timestamps are a
    /// millisecond apart and mean nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn session_record(&self, row: &SessionRow) -> AppResult<i64> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO game_session
                     (kind, app_id, app_slug, label, sandbox_id, install_id,
                      started_ms, ended_ms, seconds, exit_code, stopped_by_user)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    row.kind,
                    row.app_id,
                    row.app_slug,
                    row.label,
                    row.sandbox_id,
                    row.install_id,
                    row.started_ms,
                    row.ended_ms,
                    row.seconds,
                    row.exit_code,
                    row.stopped_by_user as i64,
                ],
            )?;

            Ok(conn.last_insert_rowid())
        })
    }

    /// Recent launches, newest first, optionally for one game.
    pub fn session_history(&self, app_id: Option<i64>, limit: usize) -> AppResult<Vec<SessionRow>> {
        let limit = limit.clamp(1, 500) as i64;

        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, kind, app_id, app_slug, label, sandbox_id, install_id,
                        started_ms, ended_ms, seconds, exit_code, stopped_by_user
                   FROM game_session
                  WHERE (?1 IS NULL OR app_id = ?1)
                  ORDER BY started_ms DESC
                  LIMIT ?2",
            )?;

            let rows = stmt
                .query_map(params![app_id, limit], session_from_row)?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(rows)
        })
    }

    /// Total play time for every game this device has launched.
    ///
    /// One grouped query rather than one per row: the Library draws a total on
    /// every game card, and a query per card is a query per frame on a screen
    /// that redraws while a download runs.
    pub fn playtime_by_app(&self) -> AppResult<std::collections::BTreeMap<i64, PlayTotals>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT app_id, SUM(seconds), COUNT(*), MAX(started_ms)
                   FROM game_session
                  WHERE app_id IS NOT NULL
                  GROUP BY app_id",
            )?;

            let mut out = std::collections::BTreeMap::new();

            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    PlayTotals {
                        seconds: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        launches: r.get::<_, i64>(2)?,
                        last_played_ms: r.get::<_, Option<i64>>(3)?,
                    },
                ))
            })?;

            for row in rows {
                let (id, totals) = row?;

                out.insert(id, totals);
            }

            Ok(out)
        })
    }

    /// Total play time per sandbox, for the sandbox list.
    pub fn playtime_by_sandbox(&self) -> AppResult<std::collections::BTreeMap<i64, PlayTotals>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT sandbox_id, SUM(seconds), COUNT(*), MAX(started_ms)
                   FROM game_session
                  WHERE sandbox_id IS NOT NULL
                  GROUP BY sandbox_id",
            )?;

            let mut out = std::collections::BTreeMap::new();

            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    PlayTotals {
                        seconds: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        launches: r.get::<_, i64>(2)?,
                        last_played_ms: r.get::<_, Option<i64>>(3)?,
                    },
                ))
            })?;

            for row in rows {
                let (id, totals) = row?;

                out.insert(id, totals);
            }

            Ok(out)
        })
    }

    /// Sessions the account has not been told about yet.
    ///
    /// The report is a separate step from the record for one reason: a launch
    /// happens while the device may be offline, and playtime that only counts
    /// when the network happened to be up is playtime that quietly goes
    /// missing. Rows are marked reported only once the API has accepted them.
    pub fn sessions_unreported(&self, limit: usize) -> AppResult<Vec<SessionRow>> {
        let limit = limit.clamp(1, 200) as i64;

        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, kind, app_id, app_slug, label, sandbox_id, install_id,
                        started_ms, ended_ms, seconds, exit_code, stopped_by_user
                   FROM game_session
                  WHERE reported = 0 AND install_id IS NOT NULL AND seconds > 0
                  ORDER BY started_ms ASC
                  LIMIT ?1",
            )?;

            let rows = stmt
                .query_map(params![limit], session_from_row)?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(rows)
        })
    }

    /// Mark sessions as reported to the account.
    pub fn sessions_mark_reported(&self, ids: &[i64]) -> AppResult<()> {
        if ids.is_empty() {
            return Ok(());
        }

        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;

            for id in ids {
                tx.execute(
                    "UPDATE game_session SET reported = 1 WHERE id = ?1",
                    params![id],
                )?;
            }

            tx.commit()?;

            Ok(())
        })
    }
}

fn session_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        app_id: r.get(2)?,
        app_slug: r.get(3)?,
        label: r.get(4)?,
        sandbox_id: r.get(5)?,
        install_id: r.get(6)?,
        started_ms: r.get(7)?,
        ended_ms: r.get(8)?,
        seconds: r.get(9)?,
        exit_code: r.get(10)?,
        stopped_by_user: r.get::<_, i64>(11)? != 0,
    })
}

impl LibraryDb {
    // ------------------------------------------------------------------ meta

    pub fn meta_get(&self, key: &str) -> AppResult<Option<String>> {
        self.with(|conn| {
            conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .optional()
        })
    }

    pub fn meta_set(&self, key: &str, value: &str) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;

            Ok(())
        })
    }

    /// Forget everything. Used on sign-out: the library is another account's
    /// business and must not survive into the next session.
    pub fn clear(&self) -> AppResult<()> {
        self.with(|conn| {
            conn.execute_batch("DELETE FROM subscription; DELETE FROM install; DELETE FROM meta;")?;

            Ok(())
        })
    }

    // ---------------------------------------------------------- subscriptions

    /// Upsert a subscription from the API.
    ///
    /// **The device-local columns are never touched here.** A sync must not
    /// forget what is installed just because the server sent the row again —
    /// which is what an `INSERT OR REPLACE` would do, and is the single easiest
    /// way to turn a working library into an endless reinstall loop.
    pub fn upsert_remote(&self, entry: &LibraryEntry) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                r#"
                INSERT INTO subscription (
                    id, kind, item_id, name, description, image, web_url,
                    app_id, app_name, app_slug,
                    auto_update, notify_updates, paused, via_collection_id,
                    installable, latest_release_id, latest_version,
                    file_url, file_name, file_size, file_sha256, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                    ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14,
                    ?15, ?16, ?17,
                    ?18, ?19, ?20, ?21, ?22
                )
                ON CONFLICT(id) DO UPDATE SET
                    kind = excluded.kind,
                    item_id = excluded.item_id,
                    name = excluded.name,
                    description = excluded.description,
                    image = excluded.image,
                    web_url = excluded.web_url,
                    app_id = excluded.app_id,
                    app_name = excluded.app_name,
                    app_slug = excluded.app_slug,
                    auto_update = excluded.auto_update,
                    notify_updates = excluded.notify_updates,
                    paused = excluded.paused,
                    via_collection_id = excluded.via_collection_id,
                    installable = excluded.installable,
                    latest_release_id = excluded.latest_release_id,
                    latest_version = excluded.latest_version,
                    file_url = excluded.file_url,
                    file_name = excluded.file_name,
                    file_size = excluded.file_size,
                    file_sha256 = excluded.file_sha256,
                    updated_at = excluded.updated_at
                "#,
                params![
                    entry.id,
                    entry.kind,
                    entry.item_id,
                    entry.name,
                    entry.description,
                    entry.image,
                    entry.web_url,
                    entry.app_id,
                    entry.app_name,
                    entry.app_slug,
                    entry.auto_update as i64,
                    entry.notify_updates as i64,
                    entry.paused as i64,
                    entry.via_collection_id,
                    entry.installable as i64,
                    entry.latest_release_id,
                    entry.latest_version,
                    entry.file_url,
                    entry.file_name,
                    entry.file_size,
                    entry.file_sha256,
                    entry.updated_at,
                ],
            )?;

            Ok(())
        })
    }

    /// Ids the server no longer lists, so a full sync can reconcile removals.
    ///
    /// Returns the rows rather than deleting them: an unsubscribed item that is
    /// still ON DISK has to be uninstalled first, and deleting the row would
    /// throw away the file list that makes that possible.
    pub fn missing_from(&self, keep: &[String]) -> AppResult<Vec<LibraryEntry>> {
        let all = self.list()?;
        let keep: std::collections::HashSet<&str> = keep.iter().map(String::as_str).collect();

        Ok(all
            .into_iter()
            .filter(|e| !keep.contains(e.id.as_str()))
            .collect())
    }

    pub fn list(&self) -> AppResult<Vec<LibraryEntry>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(SELECT_ALL)?;

            let rows = stmt.query_map([], row_to_entry)?;

            rows.collect()
        })
    }

    pub fn get(&self, id: &str) -> AppResult<Option<LibraryEntry>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!("{SELECT_ALL} WHERE id = ?1"))?;

            stmt.query_row(params![id], row_to_entry).optional()
        })
    }

    /// Find by content identity rather than subscription id.
    ///
    /// The installer works from `(kind, itemId)` because that is what a plugin
    /// rule and an install item both name; the subscription id is a server
    /// detail neither of them carries.
    pub fn find_item(&self, kind: &str, item_id: i64) -> AppResult<Option<LibraryEntry>> {
        self.with(|conn| {
            let mut stmt =
                conn.prepare(&format!("{SELECT_ALL} WHERE kind = ?1 AND item_id = ?2"))?;

            stmt.query_row(params![kind, item_id], row_to_entry)
                .optional()
        })
    }

    pub fn delete(&self, id: &str) -> AppResult<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM subscription WHERE id = ?1", params![id])?;

            Ok(())
        })
    }

    /// Record the outcome of an install.
    pub fn mark_installed(
        &self,
        id: &str,
        release_id: Option<i64>,
        version: Option<&str>,
        install_id: Option<i64>,
        files: &[String],
    ) -> AppResult<()> {
        let files = serde_json::to_string(files).unwrap_or_else(|_| "[]".into());
        let now = crate::logging::now_rfc3339();

        self.with(|conn| {
            conn.execute(
                "UPDATE subscription SET
                    installed_release_id = ?2,
                    installed_version = ?3,
                    installed_install_id = ?4,
                    installed_at = ?5,
                    installed_files = ?6,
                    state = 'installed',
                    last_error = NULL
                 WHERE id = ?1",
                params![id, release_id, version, install_id, now, files],
            )?;

            Ok(())
        })
    }

    /// Record that it is no longer on disk. The subscription row survives —
    /// the user may still be subscribed and simply have it paused.
    pub fn mark_uninstalled(&self, id: &str) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE subscription SET
                    installed_release_id = NULL,
                    installed_version = NULL,
                    installed_install_id = NULL,
                    installed_at = NULL,
                    installed_files = '[]',
                    state = 'idle',
                    last_error = NULL
                 WHERE id = ?1",
                params![id],
            )?;

            Ok(())
        })
    }

    pub fn set_state(&self, id: &str, state: &str, error: Option<&str>) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE subscription SET state = ?2, last_error = ?3 WHERE id = ?1",
                params![id, state, error],
            )?;

            Ok(())
        })
    }

    /// Reset any row left mid-flight by a crash.
    ///
    /// Called on launch. A row stuck at `installing` is not a state anything
    /// can act on — the process that owned it is gone — and leaving it there
    /// means the item never installs again and the UI shows a spinner forever.
    pub fn reset_transient_states(&self) -> AppResult<usize> {
        self.with(|conn| {
            conn.execute(
                "UPDATE subscription
                 SET state = CASE WHEN installed_release_id IS NULL THEN 'idle' ELSE 'installed' END,
                     last_error = 'Interrupted — the app closed during this operation.'
                 WHERE state IN ('installing', 'queued', 'removing')",
                [],
            )
        })
    }
}

// --------------------------------------------------------------------- installs

/// One cloud install, as the mirror stores it.
///
/// A struct rather than nine positional arguments: `upsert_install(id, app_id,
/// slug, name, is_default, version, loader, updated, payload)` has three
/// `Option<&str>` in a row, and transposing two of them compiles cleanly and is
/// only noticed when a launch uses the wrong loader.
#[derive(Debug, Clone)]
pub struct InstallMirror<'a> {
    pub id: i64,
    pub app_id: i64,
    pub app_slug: Option<&'a str>,
    pub name: &'a str,
    pub is_default: bool,
    pub game_version: Option<&'a str>,
    pub loader: Option<&'a str>,
    pub updated_at: &'a str,
    /// The API payload verbatim, so the UI parses it with the same zod schema
    /// the network path uses rather than a second Rust shape that can drift.
    pub payload: &'a str,
}

impl LibraryDb {
    /// Mirror one install from the cloud, keeping this device's `local_dir`.
    pub fn upsert_install(&self, install: &InstallMirror<'_>) -> AppResult<()> {
        let InstallMirror {
            id,
            app_id,
            app_slug,
            name,
            is_default,
            game_version,
            loader,
            updated_at,
            payload,
        } = *install;

        self.with(|conn| {
            conn.execute(
                r#"
                INSERT INTO install (
                    id, app_id, app_slug, name, is_default, game_version, loader,
                    updated_at, payload
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(id) DO UPDATE SET
                    app_id = excluded.app_id,
                    app_slug = excluded.app_slug,
                    name = excluded.name,
                    is_default = excluded.is_default,
                    game_version = excluded.game_version,
                    loader = excluded.loader,
                    updated_at = excluded.updated_at,
                    payload = excluded.payload
                "#,
                params![
                    id,
                    app_id,
                    app_slug,
                    name,
                    is_default as i64,
                    game_version,
                    loader,
                    updated_at,
                    payload
                ],
            )?;

            Ok(())
        })
    }

    /// The raw cloud payloads, for the UI. Parsing is the caller's business —
    /// this database does not need to understand an install to cache one.
    pub fn install_payloads(&self) -> AppResult<Vec<String>> {
        self.with(|conn| {
            let mut stmt =
                conn.prepare("SELECT payload FROM install ORDER BY is_default DESC, name ASC")?;

            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;

            rows.collect()
        })
    }

    /// Every mirrored install's local directory, as `(id, dir)`.
    ///
    /// One query rather than one per card: the installs screen draws all of
    /// them, and the answer is two columns.
    pub fn install_local_dirs(&self) -> AppResult<Vec<(i64, Option<String>)>> {
        self.with(|conn| {
            let mut stmt = conn.prepare("SELECT id, local_dir FROM install")?;

            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
            })?;

            rows.collect()
        })
    }

    pub fn install_local_dir(&self, id: i64) -> AppResult<Option<String>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT local_dir FROM install WHERE id = ?1",
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map(Option::flatten)
        })
    }

    pub fn set_install_local_dir(&self, id: i64, dir: Option<&str>) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE install SET local_dir = ?2 WHERE id = ?1",
                params![id, dir],
            )?;

            Ok(())
        })
    }

    pub fn retain_installs(&self, keep: &[i64]) -> AppResult<()> {
        self.with(|conn| {
            if keep.is_empty() {
                conn.execute("DELETE FROM install", [])?;

                return Ok(());
            }

            // Built rather than bound: rusqlite has no array binding, and these
            // are integers we produced, so there is nothing to inject.
            let list = keep
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");

            conn.execute(&format!("DELETE FROM install WHERE id NOT IN ({list})"), [])?;

            Ok(())
        })
    }
}

const SELECT_ALL: &str = "SELECT
    id, kind, item_id, name, description, image, web_url,
    app_id, app_name, app_slug,
    auto_update, notify_updates, paused, via_collection_id,
    installable, latest_release_id, latest_version,
    file_url, file_name, file_size, file_sha256, updated_at,
    installed_release_id, installed_version, installed_install_id,
    installed_at, installed_files, state, last_error
 FROM subscription";

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryEntry> {
    let files: String = row.get(26)?;

    Ok(LibraryEntry {
        id: row.get(0)?,
        kind: row.get(1)?,
        item_id: row.get(2)?,
        name: row.get(3)?,
        description: row.get(4)?,
        image: row.get(5)?,
        web_url: row.get(6)?,
        app_id: row.get(7)?,
        app_name: row.get(8)?,
        app_slug: row.get(9)?,
        auto_update: row.get::<_, i64>(10)? != 0,
        notify_updates: row.get::<_, i64>(11)? != 0,
        paused: row.get::<_, i64>(12)? != 0,
        via_collection_id: row.get(13)?,
        installable: row.get::<_, i64>(14)? != 0,
        latest_release_id: row.get(15)?,
        latest_version: row.get(16)?,
        file_url: row.get(17)?,
        file_name: row.get(18)?,
        file_size: row.get(19)?,
        file_sha256: row.get(20)?,
        updated_at: row.get(21)?,
        installed_release_id: row.get(22)?,
        installed_version: row.get(23)?,
        installed_install_id: row.get(24)?,
        installed_at: row.get(25)?,
        // A corrupt list degrades to empty rather than failing the whole read:
        // the worst case is an uninstall that leaves files behind and says so,
        // which beats a library that will not load.
        installed_files: serde_json::from_str(&files).unwrap_or_default(),
        state: row.get(27)?,
        last_error: row.get(28)?,
    })
}

/// A game this device installed from TMC, as the database holds it.
///
/// The row is the ONLY record that the files on disk are ours. There is no
/// rescanning our way back to it for the same reason the deployment ledger
/// cannot be rebuilt by looking at a game folder: a directory under our data
/// path could have been put there by anything, and guessing wrong either
/// strands an install forever or deletes somebody's files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledGame {
    pub app_id: i64,
    pub slug: Option<String>,
    pub name: String,
    /// The build target this was installed for — see `games::BuildPlatform`.
    pub platform: String,
    pub version: String,
    /// Absolute install directory.
    pub dir: String,
    /// The executable, relative to `dir`. Null only for a single-file build,
    /// where the artifact IS the executable.
    pub entry: Option<String>,
    /// The launch argument template, one element per argument.
    pub args: Vec<String>,
    pub size_bytes: i64,
    pub installed_ms: i64,
    pub updated_ms: i64,
    pub auto_update: bool,
}

impl LibraryDb {
    // ---------------------------------------------------------- native games

    fn game_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledGame> {
        let args: String = row.get(7)?;

        Ok(InstalledGame {
            app_id: row.get(0)?,
            slug: row.get(1)?,
            name: row.get(2)?,
            platform: row.get(3)?,
            version: row.get(4)?,
            dir: row.get(5)?,
            entry: row.get(6)?,
            /*
             * A template that will not parse degrades to "no arguments" rather
             * than failing the read, the same way `installed_files` does above:
             * a row nobody can list is a game nobody can uninstall, and the
             * launch it produces is merely one that starts at the game's own
             * menu instead of in a server.
             */
            args: serde_json::from_str(&args).unwrap_or_default(),
            size_bytes: row.get(8)?,
            installed_ms: row.get(9)?,
            updated_ms: row.get(10)?,
            auto_update: row.get::<_, i64>(11)? != 0,
        })
    }

    const GAME_COLUMNS: &'static str = "app_id, slug, name, platform, version, dir, entry, args,
                size_bytes, installed_ms, updated_ms, auto_update";

    pub fn games(&self) -> AppResult<Vec<InstalledGame>> {
        self.with(|conn| {
            let sql = format!(
                "SELECT {} FROM native_game ORDER BY name COLLATE NOCASE",
                Self::GAME_COLUMNS
            );

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], Self::game_row)?;

            rows.collect::<rusqlite::Result<Vec<_>>>()
        })
    }

    pub fn game(&self, app_id: i64) -> AppResult<Option<InstalledGame>> {
        self.with(|conn| {
            let sql = format!(
                "SELECT {} FROM native_game WHERE app_id = ?1",
                Self::GAME_COLUMNS
            );

            conn.query_row(&sql, params![app_id], Self::game_row)
                .optional()
        })
    }

    /// Write an install, replacing whatever was there.
    ///
    /// `INSERT OR REPLACE` is correct here and would be a bug on `subscription`:
    /// there are no device-local columns to lose, because every column IS a
    /// device-local fact. An update genuinely does replace all of them.
    ///
    /// `installed_ms` is preserved across an update, so "installed on the 3rd,
    /// updated yesterday" stays true — a column that reset on every update
    /// would only ever be able to say "yesterday".
    pub fn game_put(&self, game: &InstalledGame) -> AppResult<()> {
        self.with(|conn| {
            let args = serde_json::to_string(&game.args).unwrap_or_else(|_| "[]".into());

            conn.execute(
                "INSERT INTO native_game
                     (app_id, slug, name, platform, version, dir, entry, args,
                      size_bytes, installed_ms, updated_ms, auto_update)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                         COALESCE((SELECT installed_ms FROM native_game WHERE app_id = ?1), ?10),
                         ?11, ?12)
                 ON CONFLICT(app_id) DO UPDATE SET
                     slug = excluded.slug,
                     name = excluded.name,
                     platform = excluded.platform,
                     version = excluded.version,
                     dir = excluded.dir,
                     entry = excluded.entry,
                     args = excluded.args,
                     size_bytes = excluded.size_bytes,
                     updated_ms = excluded.updated_ms,
                     auto_update = excluded.auto_update",
                params![
                    game.app_id,
                    game.slug,
                    game.name,
                    game.platform,
                    game.version,
                    game.dir,
                    game.entry,
                    args,
                    game.size_bytes,
                    game.installed_ms,
                    game.updated_ms,
                    game.auto_update as i64,
                ],
            )?;

            Ok(())
        })
    }

    pub fn game_set_auto_update(&self, app_id: i64, on: bool) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE native_game SET auto_update = ?2 WHERE app_id = ?1",
                params![app_id, on as i64],
            )?;

            Ok(())
        })
    }

    /// Forget an install. The FILES are removed by the caller.
    ///
    /// Split deliberately: the row is deleted last, after the directory is
    /// gone, so a failure halfway leaves a row pointing at a partly-removed
    /// install — which the app can see and offer to finish — rather than an
    /// orphaned directory nothing knows about.
    pub fn game_delete(&self, app_id: i64) -> AppResult<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM native_game WHERE app_id = ?1", params![app_id])?;

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> LibraryEntry {
        LibraryEntry {
            id: id.into(),
            kind: "mod".into(),
            item_id: 42,
            name: "Cool Mod".into(),
            description: None,
            image: None,
            web_url: "https://example.com/m/42".into(),
            app_id: Some(1),
            app_name: Some("Minecraft".into()),
            app_slug: Some("minecraft".into()),
            auto_update: true,
            notify_updates: true,
            paused: false,
            via_collection_id: None,
            installable: true,
            latest_release_id: Some(7),
            latest_version: Some("1.4".into()),
            file_url: Some("https://example.com/download/abc".into()),
            file_name: Some("cool.jar".into()),
            file_size: Some(1024),
            file_sha256: None,
            updated_at: "2026-01-01T00:00:00Z".into(),
            installed_release_id: None,
            installed_version: None,
            installed_install_id: None,
            installed_at: None,
            installed_files: vec![],
            state: "idle".into(),
            last_error: None,
        }
    }

    #[test]
    fn a_resync_does_not_forget_what_is_installed() {
        let db = LibraryDb::open_memory().expect("db");

        db.upsert_remote(&entry("s1")).expect("insert");
        db.mark_installed("s1", Some(7), Some("1.4"), None, &["mods/cool.jar".into()])
            .expect("installed");

        // The server sends the row again, with a newer release.
        let mut next = entry("s1");
        next.latest_release_id = Some(9);
        next.latest_version = Some("1.5".into());

        db.upsert_remote(&next).expect("upsert");

        let row = db.get("s1").expect("get").expect("present");

        assert_eq!(row.latest_release_id, Some(9));
        // The device-local half survived.
        assert_eq!(row.installed_release_id, Some(7));
        assert_eq!(row.installed_files, vec!["mods/cool.jar".to_string()]);
        assert_eq!(row.state, "installed");
        assert!(row.update_available());
    }

    #[test]
    fn update_detection_uses_release_ids_not_version_strings() {
        let mut row = entry("s1");

        row.installed_release_id = Some(7);
        row.installed_version = Some("1.10".into());
        row.latest_release_id = Some(7);
        // A version string that sorts EARLIER lexically must not read as an
        // update when the release is the same one.
        row.latest_version = Some("1.9".into());

        assert!(!row.update_available());

        row.latest_release_id = Some(8);

        assert!(row.update_available());
    }

    #[test]
    fn nothing_installed_is_not_an_update() {
        let row = entry("s1");

        assert_eq!(row.installed_release_id, None);
        assert!(!row.update_available());
    }

    #[test]
    fn a_crash_mid_install_does_not_leave_a_stuck_row() {
        let db = LibraryDb::open_memory().expect("db");

        db.upsert_remote(&entry("s1")).expect("insert");
        db.set_state("s1", "installing", None).expect("state");

        db.upsert_remote(&entry("s2")).expect("insert");
        db.mark_installed("s2", Some(7), Some("1.4"), None, &[])
            .expect("installed");
        db.set_state("s2", "removing", None).expect("state");

        let reset = db.reset_transient_states().expect("reset");

        assert_eq!(reset, 2);
        // Never installed → idle. Was installed → back to installed.
        assert_eq!(db.get("s1").unwrap().unwrap().state, "idle");
        assert_eq!(db.get("s2").unwrap().unwrap().state, "installed");
    }

    #[test]
    fn missing_from_finds_what_the_server_dropped() {
        let db = LibraryDb::open_memory().expect("db");

        db.upsert_remote(&entry("s1")).expect("insert");
        db.upsert_remote(&entry("s2")).expect("insert");

        let gone = db.missing_from(&["s1".into()]).expect("missing");

        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].id, "s2");
    }

    #[test]
    fn find_item_resolves_by_content_identity() {
        let db = LibraryDb::open_memory().expect("db");

        db.upsert_remote(&entry("s1")).expect("insert");

        assert!(db.find_item("mod", 42).expect("find").is_some());
        assert!(db.find_item("mod", 43).expect("find").is_none());
        assert!(db.find_item("asset", 42).expect("find").is_none());
    }

    #[test]
    fn a_corrupt_file_list_degrades_rather_than_failing_the_read() {
        let db = LibraryDb::open_memory().expect("db");

        db.upsert_remote(&entry("s1")).expect("insert");

        db.with(|conn| {
            conn.execute(
                "UPDATE subscription SET installed_files = 'not json' WHERE id = 's1'",
                [],
            )
        })
        .expect("corrupt");

        let row = db.get("s1").expect("get").expect("present");

        assert!(row.installed_files.is_empty());
    }

    #[test]
    fn clearing_leaves_nothing_of_the_previous_account() {
        let db = LibraryDb::open_memory().expect("db");

        db.upsert_remote(&entry("s1")).expect("insert");
        db.meta_set("revision", "123").expect("meta");

        db.clear().expect("clear");

        assert!(db.list().expect("list").is_empty());
        assert_eq!(db.meta_get("revision").expect("meta"), None);
    }
}
