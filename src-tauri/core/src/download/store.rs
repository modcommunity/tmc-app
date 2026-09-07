//! Keeping the queue across a restart.
//!
//! A download manager that forgets its queue when the app closes is a download
//! manager for small files. The case this exists for is the ordinary one: a
//! forty-gigabyte modpack, a laptop lid, and coming back to it tomorrow.
//!
//! WHAT IS PERSISTED, AND WHAT IS NOT
//! ---------------------------------
//! The *request* — id, url, destination, checksum, priority, limit, and how far
//! it got. Not the speed samples, not the ETA, not the transfer's own state:
//! those describe a transfer that is no longer happening, and showing yesterday
//! evening's 4 MB/s beside a stopped download is worse than showing nothing.
//!
//! `done` IS persisted, and it is a hint rather than a fact — the real answer
//! is the length of the `.part` file, which the manager reads when it starts
//! the transfer. Storing it means the list can show a sensible bar before
//! anything is opened.
//!
//! WHAT A RESTART DOES
//! -------------------
//! Rows that were RUNNING come back as `queued`: the process that owned them is
//! gone, exactly as with a half-finished install. Rows that were PAUSED come
//! back paused, because that was a decision somebody made. Finished rows are
//! kept for the list and swept when the user clears them.

use rusqlite::{params, OptionalExtension};

use crate::error::AppResult;
use crate::library::db::LibraryDb;

use super::{DownloadRequest, DownloadState, Status};

/// A row as it comes back from disk.
#[derive(Debug, Clone)]
pub struct StoredDownload {
    pub request: DownloadRequest,
    pub status: Status,
    pub done: u64,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub queued_at: String,
}

impl LibraryDb {
    /// Insert or update one download.
    pub fn download_save(&self, state: &DownloadState) -> AppResult<()> {
        let meta = serde_json::to_string(&state.meta).unwrap_or_else(|_| "{}".into());

        self.with(|conn| {
            conn.execute(
                r#"
                INSERT INTO download (
                    id, url, dest, label, sha256, total, done, status,
                    priority, limit_bps, attempts, error, meta,
                    queued_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                ON CONFLICT(id) DO UPDATE SET
                    url = excluded.url,
                    dest = excluded.dest,
                    label = excluded.label,
                    sha256 = excluded.sha256,
                    total = excluded.total,
                    done = excluded.done,
                    status = excluded.status,
                    priority = excluded.priority,
                    limit_bps = excluded.limit_bps,
                    attempts = excluded.attempts,
                    error = excluded.error,
                    meta = excluded.meta,
                    updated_at = excluded.updated_at
                "#,
                params![
                    state.id,
                    state.url,
                    state.dest,
                    state.label,
                    state.sha256,
                    state.total.map(|t| t as i64),
                    state.done as i64,
                    state.status.as_str(),
                    state.priority,
                    state.limit_bps.map(|l| l as i64),
                    state.attempts,
                    state.error,
                    meta,
                    state.queued_at,
                    state.updated_at,
                ],
            )?;

            Ok(())
        })
    }

    /// Everything worth restoring, oldest first.
    pub fn download_list(&self) -> AppResult<Vec<StoredDownload>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, url, dest, label, sha256, total, done, status,
                        priority, limit_bps, error, meta, queued_at
                 FROM download ORDER BY queued_at ASC",
            )?;

            let rows: rusqlite::Result<Vec<StoredDownload>> = stmt
                .query_map([], |row| {
                    let meta: String = row.get(11)?;

                    Ok(StoredDownload {
                        request: DownloadRequest {
                            id: row.get(0)?,
                            url: row.get(1)?,
                            dest: std::path::PathBuf::from(row.get::<_, String>(2)?),
                            label: row.get(3)?,
                            sha256: row.get(4)?,
                            size_hint: row.get::<_, Option<i64>>(5)?.map(|t| t as u64),
                            priority: row.get(8)?,
                            limit_bps: row.get::<_, Option<i64>>(9)?.map(|l| l as u64),
                            meta: serde_json::from_str(&meta).unwrap_or_default(),
                        },
                        // A status we do not recognise reads as queued rather
                        // than failing the load: a downgrade must not make the
                        // queue unreadable.
                        status: Status::parse(&row.get::<_, String>(7)?).unwrap_or(Status::Queued),
                        done: row.get::<_, i64>(6)? as u64,
                        total: row.get::<_, Option<i64>>(5)?.map(|t| t as u64),
                        error: row.get(10)?,
                        queued_at: row.get(12)?,
                    })
                })?
                .collect();

            rows
        })
    }

    pub fn download_delete(&self, id: &str) -> AppResult<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM download WHERE id = ?1", params![id])?;

            Ok(())
        })
    }

    /// Drop every finished row.
    pub fn download_clear_finished(&self) -> AppResult<usize> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM download WHERE status IN ('done', 'failed', 'cancelled')",
                [],
            )
        })
    }

    pub fn download_clear_all(&self) -> AppResult<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM download", [])?;

            Ok(())
        })
    }

    /// Turn any row left mid-transfer back into a queued one.
    ///
    /// Called at launch, for the same reason `reset_transient_states` is: the
    /// process that owned a `running` row is gone, so nothing will ever move it
    /// and the UI would show a stalled bar forever.
    pub fn download_reset_running(&self) -> AppResult<usize> {
        self.with(|conn| {
            conn.execute(
                "UPDATE download SET status = 'queued' WHERE status = 'running'",
                [],
            )
        })
    }

    pub fn download_get(&self, id: &str) -> AppResult<Option<String>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT status FROM download WHERE id = ?1",
                params![id],
                |r| r.get::<_, String>(0),
            )
            .optional()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn state(id: &str, status: Status) -> DownloadState {
        DownloadState {
            id: id.into(),
            label: "Cool Mod".into(),
            url: "https://cdn.test/a.jar".into(),
            dest: "/games/mc/mods/a.jar".into(),
            status,
            total: Some(1000),
            done: 400,
            speed_bps: 12345,
            eta_secs: Some(10),
            priority: 5,
            limit_bps: Some(1024),
            attempts: 1,
            error: None,
            sha256: Some("abc".into()),
            samples: vec![1, 2, 3],
            queued_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:01:00Z".into(),
            meta: BTreeMap::from([("sandboxId".into(), "3".into())]),
        }
    }

    #[test]
    fn a_download_round_trips_with_its_metadata() {
        let db = LibraryDb::open_memory().expect("db");

        db.download_save(&state("d1", Status::Paused))
            .expect("save");

        let rows = db.download_list().expect("list");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request.id, "d1");
        assert_eq!(rows[0].status, Status::Paused);
        assert_eq!(rows[0].done, 400);
        assert_eq!(rows[0].request.priority, 5);
        assert_eq!(rows[0].request.limit_bps, Some(1024));
        assert_eq!(rows[0].request.sha256.as_deref(), Some("abc"));
        assert_eq!(
            rows[0].request.meta.get("sandboxId").map(String::as_str),
            Some("3")
        );
    }

    #[test]
    fn saving_the_same_id_twice_updates_rather_than_duplicating() {
        let db = LibraryDb::open_memory().expect("db");

        db.download_save(&state("d1", Status::Running))
            .expect("save");

        let mut later = state("d1", Status::Done);
        later.done = 1000;

        db.download_save(&later).expect("save");

        let rows = db.download_list().expect("list");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Done);
        assert_eq!(rows[0].done, 1000);
    }

    /// The process that owned a running transfer is gone. Leaving the row as
    /// running means a bar that never moves and a download that never restarts.
    #[test]
    fn a_running_row_comes_back_queued() {
        let db = LibraryDb::open_memory().expect("db");

        db.download_save(&state("d1", Status::Running))
            .expect("save");
        db.download_save(&state("d2", Status::Paused))
            .expect("save");

        assert_eq!(db.download_reset_running().expect("reset"), 1);

        let rows = db.download_list().expect("list");

        let by_id = |id: &str| {
            rows.iter()
                .find(|r| r.request.id == id)
                .map(|r| r.status)
                .expect("present")
        };

        assert_eq!(by_id("d1"), Status::Queued);
        // A pause was somebody's decision and survives.
        assert_eq!(by_id("d2"), Status::Paused);
    }

    #[test]
    fn clearing_finished_leaves_the_active_ones() {
        let db = LibraryDb::open_memory().expect("db");

        for (id, status) in [
            ("a", Status::Done),
            ("b", Status::Failed),
            ("c", Status::Cancelled),
            ("d", Status::Queued),
            ("e", Status::Paused),
        ] {
            db.download_save(&state(id, status)).expect("save");
        }

        assert_eq!(db.download_clear_finished().expect("clear"), 3);

        let left: Vec<String> = db
            .download_list()
            .expect("list")
            .into_iter()
            .map(|r| r.request.id)
            .collect();

        assert_eq!(left, vec!["d".to_string(), "e".to_string()]);
    }

    /// The database outlives any one app version. A status a downgrade does not
    /// know must not make the whole queue unreadable.
    #[test]
    fn an_unknown_status_reads_as_queued_rather_than_failing_the_load() {
        let db = LibraryDb::open_memory().expect("db");

        db.download_save(&state("d1", Status::Queued))
            .expect("save");

        db.with(|conn| {
            conn.execute(
                "UPDATE download SET status = 'verifying' WHERE id = 'd1'",
                [],
            )
        })
        .expect("tamper");

        let rows = db.download_list().expect("list");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Queued);
    }
}
