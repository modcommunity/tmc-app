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
const SCHEMA_VERSION: i64 = 1;

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
    /// Which install (sandbox) it was materialised into. `None` = the default.
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
        self.installable
            && !self.paused
            && !self.is_container()
            && self.file_url.is_some()
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

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> AppResult<T> {
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
    fn migrate(&self) -> AppResult<()> {
        self.with(|conn| {
            let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

            if version >= SCHEMA_VERSION {
                return Ok(());
            }

            conn.execute_batch(
                r#"
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
                "#,
            )?;

            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;

            Ok(())
        })
    }

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
