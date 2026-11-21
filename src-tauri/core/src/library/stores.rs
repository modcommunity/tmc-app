//! **Stores**: named storage locations downloads and installs can be placed in.
//!
//! A store is a directory the user chose, with a name and a kind:
//!
//!   * **Permanent**: kept. The app never deletes anything inside it.
//!   * **Temporary**: its contents are wiped when the app exits, and again on
//!     the next launch in case the app crashed or was killed before it could.
//!
//! WHAT KEEPS THIS FROM BEING A FOOT-GUN
//! -------------------------------------
//! Wiping a directory is the most dangerous thing this app does to a path a
//! user typed, so it has three guards:
//!
//!   1. A temporary store must be an EMPTY directory when it is created (the
//!      app is not going to clear out somebody's Documents folder because they
//!      picked it in a hurry).
//!   2. Creating one writes a marker file ([`TEMP_MARKER`]) and a wipe only
//!      ever proceeds when that marker is still there. A folder the user has
//!      since emptied and reused, or a database row restored from a backup that
//!      now points somewhere else, is left alone.
//!   3. The wipe empties the folder; it never removes the folder itself, never
//!      follows a symlink, and never runs for the app's own cache (which has its
//!      own sweep at launch).
//!
//! DELETING A STORE
//! ----------------
//! A permanent store is only FORGOTTEN: the row goes, every file stays. A
//! temporary store is wiped (the user must confirm when it is not empty) and
//! the marker removed, so the folder is left empty and ordinary. Neither kind
//! can be deleted while it is the default or while a game's install folder is
//! inside it, because that would orphan an install the app still points at.
//!
//! WHAT A STORE MAY POINT AT
//! -------------------------
//! An absolute path, with no `..`, that can be created and written to; that
//! passes [`crate::anchor::validate_root`] (not a filesystem root or system
//! folder, does not contain the app's own files); that is not inside the app's
//! own data, logs, cache or plugin folders; and that neither contains nor sits
//! inside another store. The default store is what `settings.download_dir`
//! says (see [`LibraryDb::store_sync_default`]), so existing behaviour is
//! unchanged and the plugin jail's `downloads` anchor stays the one setting.

use std::io;
use std::path::{Component, Path, PathBuf};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::anchor;
use crate::canon;
use crate::error::{AppError, AppResult};
use crate::library::db::LibraryDb;
use crate::logging::now_rfc3339;

/// Cap on stores, so a runaway script cannot fill the table.
pub const MAX_STORES: usize = 50;

/// The file whose presence says "this folder is a temporary store the app
/// created and may wipe".
pub const TEMP_MARKER: &str = ".tmc-temp-store";

/// Entries examined when measuring a store, so a folder holding millions of
/// files cannot stall the page. The size is then reported as a lower bound.
const SIZE_SCAN_LIMIT: usize = 200_000;

/// How long a name may be, in characters.
const MAX_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreKind {
    Permanent,
    Temporary,
}

impl StoreKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Permanent => "permanent",
            Self::Temporary => "temporary",
        }
    }

    fn parse(text: &str) -> Self {
        if text == "temporary" {
            Self::Temporary
        } else {
            Self::Permanent
        }
    }
}

/// A store, as the UI sees it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub kind: StoreKind,
    pub is_default: bool,
    /// The app's own download cache. Cannot be renamed or deleted, and is
    /// cleared by the app at launch rather than by the temporary-store sweep.
    pub builtin: bool,
    pub created_at: String,
    /// Whether the folder is there right now (a drive may be unplugged).
    pub exists: bool,
    /// Bytes in the folder (files only, symlinks not followed).
    pub used_bytes: u64,
    /// True when the scan hit its limit and `used_bytes` is a lower bound.
    pub used_truncated: bool,
    /// Free bytes on the volume, when the platform can say.
    pub free_bytes: Option<u64>,
}

impl Store {
    /// Fill in the disk figures. Separate from loading the row so a caller
    /// that only needs the row does not walk a directory tree.
    pub fn fill_stats(&mut self) {
        let path = Path::new(&self.path);

        self.exists = path.is_dir();

        if self.exists {
            let (used, truncated) = dir_size(path, SIZE_SCAN_LIMIT);

            self.used_bytes = used;
            self.used_truncated = truncated;
            self.free_bytes = free_bytes(path);
        }
    }
}

/// What the startup and exit sweep did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Files and folders removed from temporary stores.
    pub removed: usize,
    /// Temporary stores that could not be (fully) wiped.
    pub failed: usize,
}

fn store_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Store> {
    let kind: String = row.get(3)?;
    let is_default: i64 = row.get(4)?;
    let builtin: i64 = row.get(5)?;

    Ok(Store {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        kind: StoreKind::parse(&kind),
        is_default: is_default != 0,
        builtin: builtin != 0,
        created_at: row.get(6)?,
        exists: false,
        used_bytes: 0,
        used_truncated: false,
        free_bytes: None,
    })
}

const STORE_COLUMNS: &str = "id, name, path, kind, is_default, builtin, created_at";

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn clean_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();

    if name.is_empty() {
        return Err(AppError::invalid("Give this store a name."));
    }

    if name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::invalid(format!(
            "A store name can be at most {MAX_NAME_CHARS} characters."
        )));
    }

    if name.chars().any(char::is_control) {
        return Err(AppError::invalid(
            "A store name cannot hold control characters.",
        ));
    }

    Ok(name.to_string())
}

/// Bytes in files under `root`, without following symlinks, examining at most
/// `max_entries` directory entries. The flag says the limit was hit.
pub fn dir_size(root: &Path, max_entries: usize) -> (u64, bool) {
    let mut total: u64 = 0;
    let mut seen: usize = 0;
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in read.flatten() {
            seen += 1;

            if seen > max_entries {
                return (total, true);
            }

            // `DirEntry::metadata` does not follow symlinks.
            let Ok(meta) = entry.metadata() else {
                continue;
            };

            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }

    (total, false)
}

/// Free bytes available to this user on the volume holding `path`.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)] // the field widths differ per platform
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;

    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();

    // SAFETY: `c_path` is a valid NUL-terminated string and `stat` is a
    // writable `statvfs` the call fills in on success.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };

    if rc != 0 {
        return None;
    }

    // SAFETY: `statvfs` returned 0, so the struct is initialised.
    let stat = unsafe { stat.assume_init() };

    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

/// Free space is not reported off Unix: the UI shows only the used size.
#[cfg(not(unix))]
pub fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// Empty a temporary store, leaving the folder and its marker.
///
/// Refuses (with `PermissionDenied`) unless the marker is a regular file, so
/// only a folder the app itself marked is ever wiped. Keeps going past a
/// failing entry and reports the first failure at the end.
pub fn wipe_temporary(dir: &Path) -> io::Result<usize> {
    let marker = dir.join(TEMP_MARKER);

    match std::fs::symlink_metadata(&marker) {
        Ok(meta) if meta.is_file() => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "folder is not marked as a temporary store",
            ));
        }
    }

    if dir.parent().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to wipe a filesystem root",
        ));
    }

    let mut removed = 0;
    let mut first_error: Option<io::Error> = None;

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;

        if entry.file_name().as_os_str() == std::ffi::OsStr::new(TEMP_MARKER) {
            continue;
        }

        let path = entry.path();

        // `file_type` does not follow symlinks: a link is removed as a link
        // and what it points at is never touched.
        let result = match entry.file_type() {
            Ok(kind) if kind.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(e) => Err(e),
        };

        match result {
            Ok(()) => removed += 1,
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }

    match first_error {
        Some(e) => Err(e),
        None => Ok(removed),
    }
}

/// Entries in a store other than its marker.
fn entry_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|read| {
            read.flatten()
                .filter(|e| e.file_name().as_os_str() != std::ffi::OsStr::new(TEMP_MARKER))
                .count()
        })
        .unwrap_or(0)
}

fn probe_writable(dir: &Path) -> AppResult<()> {
    let probe = dir.join(format!(".tmc-write-test-{}", std::process::id()));

    std::fs::write(&probe, b"")
        .map_err(|e| AppError::invalid(format!("That folder is not writable: {e}")))?;

    let _ = std::fs::remove_file(&probe);

    Ok(())
}

impl LibraryDb {
    /// Every store, without disk figures. See [`Store::fill_stats`].
    pub fn store_list(&self) -> AppResult<Vec<Store>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {STORE_COLUMNS} FROM storage_store ORDER BY builtin DESC, id"
            ))?;

            let rows = stmt.query_map([], store_row)?;

            rows.collect()
        })
    }

    pub fn store_get(&self, id: i64) -> AppResult<Option<Store>> {
        self.with(|conn| {
            conn.query_row(
                &format!("SELECT {STORE_COLUMNS} FROM storage_store WHERE id = ?1"),
                params![id],
                store_row,
            )
            .optional()
        })
    }

    fn store_name_taken(&self, name: &str, except_id: i64) -> AppResult<bool> {
        self.with(|conn| {
            let taken: i64 = conn.query_row(
                "SELECT COUNT(*) FROM storage_store WHERE lower(name) = lower(?1) AND id != ?2",
                params![name, except_id],
                |row| row.get(0),
            )?;

            Ok(taken > 0)
        })
    }

    /// Make the table agree with `settings.download_dir`, which stays the one
    /// source of truth for where downloads go.
    ///
    ///   * The app's own cache folder always has a (built-in) row.
    ///   * When a download folder is configured and no store has that path, a
    ///     permanent store is made for it, so a user who set one in Settings
    ///     sees it here.
    ///   * Exactly one store is the default: the one at `download_dir`, or the
    ///     built-in one when that is unset.
    pub fn store_sync_default(
        &self,
        download_dir: Option<&str>,
        cache_downloads: &Path,
    ) -> AppResult<()> {
        let cache = path_text(&canon::canonicalize_or_keep(cache_downloads));
        let now = now_rfc3339();

        self.with(|conn| {
            let builtin: Option<i64> = conn
                .query_row(
                    "SELECT id FROM storage_store WHERE builtin = 1",
                    [],
                    |row| row.get(0),
                )
                .optional()?;

            let builtin_id = match builtin {
                Some(id) => {
                    conn.execute(
                        "UPDATE storage_store SET path = ?1 WHERE id = ?2 AND path != ?1",
                        params![cache, id],
                    )?;

                    id
                }
                None => {
                    conn.execute(
                        r#"
                        INSERT OR IGNORE INTO storage_store
                            (name, path, kind, is_default, builtin, created_at)
                        VALUES ('App downloads', ?1, 'temporary', 0, 1, ?2)
                        "#,
                        params![cache, now],
                    )?;

                    conn.query_row(
                        "SELECT id FROM storage_store WHERE path = ?1",
                        params![cache],
                        |row| row.get(0),
                    )?
                }
            };

            let target_id = match download_dir {
                None => builtin_id,
                Some(dir) => {
                    let path = path_text(&canon::canonicalize_or_keep(dir));

                    conn.execute(
                        r#"
                        INSERT OR IGNORE INTO storage_store
                            (name, path, kind, is_default, builtin, created_at)
                        VALUES ('Downloads folder', ?1, 'permanent', 0, 0, ?2)
                        "#,
                        params![path, now],
                    )?;

                    conn.query_row(
                        "SELECT id FROM storage_store WHERE path = ?1",
                        params![path],
                        |row| row.get(0),
                    )?
                }
            };

            conn.execute(
                "UPDATE storage_store SET is_default = (id = ?1)",
                params![target_id],
            )?;

            Ok(())
        })
    }

    /// Create a store at `raw_path`, making the folder when it is missing.
    ///
    /// `protected` is what [`anchor::validate_root`] takes (the app's own
    /// folders and the user's home, which a store may not contain);
    /// `app_owned` is the app's own folders, which a store may not be INSIDE.
    pub fn store_create(
        &self,
        name: &str,
        raw_path: &str,
        kind: StoreKind,
        protected: &[PathBuf],
        app_owned: &[PathBuf],
    ) -> AppResult<i64> {
        let name = clean_name(name)?;

        if self.store_name_taken(&name, 0)? {
            return Err(AppError::invalid("A store with that name already exists."));
        }

        if self.store_list()?.len() >= MAX_STORES {
            return Err(AppError::invalid(format!(
                "You already have {MAX_STORES} stores."
            )));
        }

        let given = Path::new(raw_path.trim());

        if !given.is_absolute() {
            return Err(AppError::invalid(
                "Choose a full folder path, starting at a drive or the filesystem root.",
            ));
        }

        if given
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(AppError::invalid("A store path cannot contain `..`."));
        }

        let created = !given.exists();

        if created {
            std::fs::create_dir_all(given)
                .map_err(|e| AppError::invalid(format!("Could not create that folder: {e}")))?;
        }

        let result = self.store_create_checked(&name, given, kind, protected, app_owned);

        if result.is_err() && created {
            // Only removes an empty folder, which is all a failed create left.
            let _ = std::fs::remove_dir(given);
        }

        result
    }

    fn store_create_checked(
        &self,
        name: &str,
        given: &Path,
        kind: StoreKind,
        protected: &[PathBuf],
        app_owned: &[PathBuf],
    ) -> AppResult<i64> {
        let dir = anchor::validate_root(&path_text(given), protected)?;

        for owned in app_owned {
            if dir.starts_with(canon::canonicalize_or_keep(owned)) {
                return Err(AppError::invalid(
                    "That folder is inside the app's own data. Choose somewhere else.",
                ));
            }
        }

        for other in self.store_list()? {
            let theirs = canon::canonicalize_or_keep(&other.path);

            if dir.starts_with(&theirs) || theirs.starts_with(&dir) {
                return Err(AppError::invalid(format!(
                    "That folder overlaps the store \"{}\" ({}). Stores cannot be nested.",
                    other.name, other.path
                )));
            }
        }

        probe_writable(&dir)?;

        if kind == StoreKind::Temporary {
            if entry_count(&dir) > 0 {
                return Err(AppError::invalid(
                    "A temporary store is emptied when the app closes, so it must be an empty folder.",
                ));
            }

            std::fs::write(
                dir.join(TEMP_MARKER),
                b"GameSpice temporary store: emptied when the app closes.\n",
            )
            .map_err(|e| AppError::invalid(format!("That folder is not writable: {e}")))?;
        }

        let path = path_text(&dir);
        let now = now_rfc3339();

        self.with(|conn| {
            conn.execute(
                r#"
                INSERT INTO storage_store (name, path, kind, is_default, builtin, created_at)
                VALUES (?1, ?2, ?3, 0, 0, ?4)
                "#,
                params![name, path, kind.as_str(), now],
            )?;

            Ok(conn.last_insert_rowid())
        })
    }

    pub fn store_rename(&self, id: i64, name: &str) -> AppResult<()> {
        let name = clean_name(name)?;
        let store = self
            .store_get(id)?
            .ok_or_else(|| AppError::invalid("That store no longer exists."))?;

        if store.builtin {
            return Err(AppError::invalid(
                "The app's own download folder cannot be renamed.",
            ));
        }

        if self.store_name_taken(&name, id)? {
            return Err(AppError::invalid("A store with that name already exists."));
        }

        self.with(|conn| {
            conn.execute(
                "UPDATE storage_store SET name = ?1 WHERE id = ?2",
                params![name, id],
            )?;

            Ok(())
        })
    }

    /// What `settings.download_dir` must become for store `id` to be the
    /// default: `Some(path)`, or `None` for the built-in store (reset to the
    /// app cache). The caller applies it through the validating setter and
    /// then calls [`Self::store_sync_default`].
    ///
    /// A temporary store cannot be the default, because everything installed
    /// by default would vanish when the app closed.
    pub fn store_default_target(&self, id: i64) -> AppResult<Option<String>> {
        let store = self
            .store_get(id)?
            .ok_or_else(|| AppError::invalid("That store no longer exists."))?;

        if store.builtin {
            return Ok(None);
        }

        if store.kind == StoreKind::Temporary {
            return Err(AppError::invalid(
                "A temporary store cannot be the default: its files are wiped when the app closes.",
            ));
        }

        if !Path::new(&store.path).is_dir() {
            return Err(AppError::invalid("That store's folder is missing."));
        }

        Ok(Some(store.path))
    }

    /// Delete a store. See the module header for what this does and does not
    /// remove: a permanent store is only forgotten.
    ///
    /// `game_dirs` is every configured game install folder, so a store holding
    /// an install is refused rather than orphaned.
    pub fn store_delete(&self, id: i64, confirm_wipe: bool, game_dirs: &[String]) -> AppResult<()> {
        let store = self
            .store_get(id)?
            .ok_or_else(|| AppError::invalid("That store no longer exists."))?;

        if store.builtin {
            return Err(AppError::invalid(
                "The app's own download folder cannot be deleted.",
            ));
        }

        if store.is_default {
            return Err(AppError::invalid(
                "This is the default store. Make another store the default first.",
            ));
        }

        let root = canon::canonicalize_or_keep(&store.path);

        if game_dirs
            .iter()
            .any(|dir| canon::canonicalize_or_keep(dir).starts_with(&root))
        {
            return Err(AppError::invalid(
                "A game is installed in this store. Move or reset its install folder (Settings > Games) first.",
            ));
        }

        let dir = Path::new(&store.path);

        if store.kind == StoreKind::Temporary && dir.join(TEMP_MARKER).is_file() {
            if entry_count(dir) > 0 && !confirm_wipe {
                return Err(AppError::invalid(
                    "This temporary store is not empty. Deleting it erases what is inside; confirm to continue.",
                ));
            }

            wipe_temporary(dir)
                .map_err(|e| AppError::invalid(format!("Could not empty the store: {e}")))?;

            // Leave an ordinary empty folder behind.
            let _ = std::fs::remove_file(dir.join(TEMP_MARKER));
        }

        self.with(|conn| {
            conn.execute("DELETE FROM storage_store WHERE id = ?1", params![id])?;

            Ok(())
        })
    }

    /// Empty every temporary store. Run at startup (catching a crash) and at
    /// exit. The built-in cache store is skipped: the app clears that itself.
    pub fn store_sweep_temporary(&self) -> AppResult<SweepReport> {
        let mut report = SweepReport::default();

        for store in self.store_list()? {
            if store.builtin || store.kind != StoreKind::Temporary {
                continue;
            }

            let dir = Path::new(&store.path);

            if !dir.is_dir() {
                continue;
            }

            match wipe_temporary(dir) {
                Ok(n) => report.removed += n,
                Err(_) => report.failed += 1,
            }
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> LibraryDb {
        LibraryDb::open_memory().expect("db")
    }

    fn make(db: &LibraryDb, tmp: &Path, name: &str, rel: &str, kind: StoreKind) -> AppResult<i64> {
        db.store_create(name, &path_text(&tmp.join(rel)), kind, &[], &[])
    }

    #[test]
    fn a_permanent_store_is_created_listed_and_canonical() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();

        let id = make(&db, tmp.path(), "Big disk", "big", StoreKind::Permanent).expect("create");
        let listed = db.store_list().expect("list");

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, id);
        assert_eq!(listed[0].kind, StoreKind::Permanent);
        assert!(Path::new(&listed[0].path).is_dir());
        assert!(!listed[0].is_default);
    }

    #[test]
    fn a_relative_or_parent_path_is_refused() {
        let db = db();

        assert!(db
            .store_create("x", "relative/dir", StoreKind::Permanent, &[], &[])
            .is_err());

        let tmp = tempfile::tempdir().expect("tmp");
        let sneaky = format!("{}/a/../b", tmp.path().display());

        assert!(db
            .store_create("x", &sneaky, StoreKind::Permanent, &[], &[])
            .is_err());
        assert!(!tmp.path().join("b").exists());
    }

    #[test]
    fn names_must_be_present_and_unique() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();

        assert!(make(&db, tmp.path(), "   ", "a", StoreKind::Permanent).is_err());

        make(&db, tmp.path(), "Disk", "a", StoreKind::Permanent).expect("first");

        assert!(make(&db, tmp.path(), "disk", "b", StoreKind::Permanent).is_err());
    }

    #[test]
    fn stores_cannot_be_nested_either_way() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();

        make(&db, tmp.path(), "Outer", "outer", StoreKind::Permanent).expect("outer");

        assert!(make(
            &db,
            tmp.path(),
            "Inner",
            "outer/inner",
            StoreKind::Permanent
        )
        .is_err());

        make(&db, tmp.path(), "Deep", "x/deep", StoreKind::Permanent).expect("deep");

        assert!(make(&db, tmp.path(), "Parent", "x", StoreKind::Permanent).is_err());
    }

    #[test]
    fn a_store_inside_the_apps_own_folder_is_refused_and_not_left_behind() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();
        let data = tmp.path().join("appdata");

        std::fs::create_dir_all(&data).expect("data");

        let target = data.join("stores");
        let result = db.store_create(
            "Sneaky",
            &path_text(&target),
            StoreKind::Permanent,
            &[],
            std::slice::from_ref(&data),
        );

        assert!(result.is_err());
        assert!(!target.exists());
    }

    #[test]
    fn a_store_containing_the_apps_own_folder_is_refused() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();
        let data = tmp.path().join("outer/appdata");

        std::fs::create_dir_all(&data).expect("data");

        let result = db.store_create(
            "Too wide",
            &path_text(&tmp.path().join("outer")),
            StoreKind::Permanent,
            std::slice::from_ref(&data),
            &[],
        );

        assert!(result.is_err());
    }

    #[test]
    fn a_temporary_store_needs_an_empty_folder_and_is_marked() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();
        let full = tmp.path().join("full");

        std::fs::create_dir_all(&full).expect("dir");
        std::fs::write(full.join("keep.txt"), b"mine").expect("file");

        assert!(db
            .store_create("Temp", &path_text(&full), StoreKind::Temporary, &[], &[])
            .is_err());
        assert!(full.join("keep.txt").exists());
        assert!(!full.join(TEMP_MARKER).exists());

        make(&db, tmp.path(), "Scratch", "scratch", StoreKind::Temporary).expect("create");

        assert!(tmp.path().join("scratch").join(TEMP_MARKER).is_file());
    }

    #[test]
    fn the_sweep_empties_temporary_stores_only() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();

        make(&db, tmp.path(), "Scratch", "scratch", StoreKind::Temporary).expect("temp");
        make(&db, tmp.path(), "Keep", "keep", StoreKind::Permanent).expect("perm");

        let scratch = tmp.path().join("scratch");
        let keep = tmp.path().join("keep");

        std::fs::write(scratch.join("a.bin"), b"1").expect("a");
        std::fs::create_dir_all(scratch.join("sub/deeper")).expect("sub");
        std::fs::write(scratch.join("sub/deeper/b.bin"), b"2").expect("b");
        std::fs::write(keep.join("c.bin"), b"3").expect("c");

        let report = db.store_sweep_temporary().expect("sweep");

        assert_eq!(report.removed, 2);
        assert_eq!(report.failed, 0);
        assert!(scratch.is_dir());
        assert!(scratch.join(TEMP_MARKER).is_file());
        assert!(!scratch.join("a.bin").exists());
        assert!(!scratch.join("sub").exists());
        assert!(keep.join("c.bin").exists());

        // Idempotent: a second sweep (startup after exit) finds nothing.
        assert_eq!(db.store_sweep_temporary().expect("again").removed, 0);
    }

    #[test]
    fn the_sweep_refuses_a_folder_that_lost_its_marker() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();

        make(&db, tmp.path(), "Scratch", "scratch", StoreKind::Temporary).expect("temp");

        let scratch = tmp.path().join("scratch");

        std::fs::remove_file(scratch.join(TEMP_MARKER)).expect("marker");
        std::fs::write(scratch.join("precious.txt"), b"x").expect("file");

        let report = db.store_sweep_temporary().expect("sweep");

        assert_eq!(report.failed, 1);
        assert!(scratch.join("precious.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn the_wipe_removes_a_symlink_but_not_its_target() {
        let tmp = tempfile::tempdir().expect("tmp");
        let outside = tmp.path().join("outside");
        let scratch = tmp.path().join("scratch");

        std::fs::create_dir_all(&outside).expect("outside");
        std::fs::write(outside.join("safe.txt"), b"x").expect("safe");
        std::fs::create_dir_all(&scratch).expect("scratch");
        std::fs::write(scratch.join(TEMP_MARKER), b"").expect("marker");
        std::os::unix::fs::symlink(&outside, scratch.join("link")).expect("link");

        wipe_temporary(&scratch).expect("wipe");

        assert!(!scratch.join("link").exists());
        assert!(outside.join("safe.txt").exists());
    }

    #[test]
    fn sync_seeds_the_builtin_store_as_default() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();
        let cache = tmp.path().join("cache/downloads");

        db.store_sync_default(None, &cache).expect("sync");
        db.store_sync_default(None, &cache).expect("again");

        let listed = db.store_list().expect("list");

        assert_eq!(listed.len(), 1);
        assert!(listed[0].builtin);
        assert!(listed[0].is_default);
        assert_eq!(listed[0].kind, StoreKind::Temporary);

        // The built-in store is never swept by the temporary-store sweep.
        std::fs::create_dir_all(&cache).expect("cache");
        std::fs::write(cache.join("x"), b"1").expect("x");

        assert_eq!(db.store_sweep_temporary().expect("sweep").removed, 0);
    }

    #[test]
    fn sync_follows_the_configured_download_folder() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();
        let cache = tmp.path().join("cache/downloads");
        let dl = tmp.path().join("dl");

        std::fs::create_dir_all(&dl).expect("dl");

        db.store_sync_default(Some(&path_text(&dl)), &cache)
            .expect("sync");

        let listed = db.store_list().expect("list");
        let default = listed.iter().find(|s| s.is_default).expect("default");

        assert_eq!(listed.len(), 2);
        assert_eq!(default.kind, StoreKind::Permanent);
        assert!(!default.builtin);
        assert_eq!(listed.iter().filter(|s| s.is_default).count(), 1);

        // Back to the app cache.
        db.store_sync_default(None, &cache).expect("reset");

        assert!(
            db.store_list()
                .expect("list")
                .iter()
                .find(|s| s.is_default)
                .expect("default")
                .builtin
        );
    }

    #[test]
    fn a_temporary_store_cannot_become_the_default() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();

        let temp = make(&db, tmp.path(), "Scratch", "s", StoreKind::Temporary).expect("temp");
        let perm = make(&db, tmp.path(), "Keep", "k", StoreKind::Permanent).expect("perm");

        assert!(db.store_default_target(temp).is_err());
        assert!(db.store_default_target(perm).expect("perm").is_some());
    }

    #[test]
    fn deleting_a_permanent_store_forgets_it_and_keeps_every_file() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();
        let id = make(&db, tmp.path(), "Keep", "keep", StoreKind::Permanent).expect("create");

        std::fs::write(tmp.path().join("keep/game.bin"), b"x").expect("file");

        db.store_delete(id, false, &[]).expect("delete");

        assert!(db.store_list().expect("list").is_empty());
        assert!(tmp.path().join("keep/game.bin").exists());
    }

    #[test]
    fn deleting_refuses_the_default_the_builtin_and_a_store_holding_an_install() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();
        let cache = tmp.path().join("cache/downloads");
        let dl = tmp.path().join("dl");

        std::fs::create_dir_all(&dl).expect("dl");
        db.store_sync_default(Some(&path_text(&dl)), &cache)
            .expect("sync");

        let listed = db.store_list().expect("list");
        let builtin = listed.iter().find(|s| s.builtin).expect("builtin").id;
        let default = listed.iter().find(|s| s.is_default).expect("default").id;

        assert!(db.store_delete(builtin, false, &[]).is_err());
        assert!(db.store_delete(default, false, &[]).is_err());

        let id = make(&db, tmp.path(), "Games", "games", StoreKind::Permanent).expect("create");
        let install = tmp.path().join("games/rust");

        std::fs::create_dir_all(&install).expect("install");

        assert!(db.store_delete(id, false, &[path_text(&install)]).is_err());
        assert!(db.store_delete(id, false, &[]).is_ok());
    }

    #[test]
    fn deleting_a_non_empty_temporary_store_needs_confirmation() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();
        let id = make(&db, tmp.path(), "Scratch", "s", StoreKind::Temporary).expect("create");
        let dir = tmp.path().join("s");

        std::fs::write(dir.join("a.bin"), b"1").expect("file");

        assert!(db.store_delete(id, false, &[]).is_err());
        assert!(dir.join("a.bin").exists());

        db.store_delete(id, true, &[]).expect("confirmed");

        assert!(dir.is_dir());
        assert!(!dir.join("a.bin").exists());
        assert!(!dir.join(TEMP_MARKER).exists());
        assert!(db.store_list().expect("list").is_empty());
    }

    #[test]
    fn rename_validates_and_refuses_the_builtin() {
        let tmp = tempfile::tempdir().expect("tmp");
        let db = db();

        db.store_sync_default(None, &tmp.path().join("c"))
            .expect("sync");

        let builtin = db.store_list().expect("list")[0].id;
        let a = make(&db, tmp.path(), "A", "a", StoreKind::Permanent).expect("a");

        make(&db, tmp.path(), "B", "b", StoreKind::Permanent).expect("b");

        assert!(db.store_rename(builtin, "Mine").is_err());
        assert!(db.store_rename(a, "b").is_err());
        assert!(db.store_rename(a, "  ").is_err());
        assert!(db.store_rename(a, "Renamed").is_ok());
        assert_eq!(db.store_get(a).expect("get").expect("row").name, "Renamed");
    }

    #[test]
    fn size_counts_files_and_reports_truncation() {
        let tmp = tempfile::tempdir().expect("tmp");

        std::fs::create_dir_all(tmp.path().join("d")).expect("d");
        std::fs::write(tmp.path().join("a"), vec![0u8; 10]).expect("a");
        std::fs::write(tmp.path().join("d/b"), vec![0u8; 20]).expect("b");

        assert_eq!(dir_size(tmp.path(), 100), (30, false));
        assert!(dir_size(tmp.path(), 1).1);
    }

    #[cfg(unix)]
    #[test]
    fn free_space_is_reported_on_unix() {
        let tmp = tempfile::tempdir().expect("tmp");

        assert!(free_bytes(tmp.path()).is_some());
    }
}
