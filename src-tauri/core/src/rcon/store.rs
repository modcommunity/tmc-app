//! Saved RCON servers, with their passwords encrypted.
//!
//! **Nothing here is ever sent to the website.** A server's RCON password is
//! not derived from a TMC account, losing it costs somebody their server rather
//! than their profile, and no feature on the site needs it — so it stays on the
//! device that will use it. That is a deliberate exception to the rule that a
//! user's configuration syncs; it is the same reasoning that keeps game
//! directories local, applied to something with a much higher cost of being
//! wrong.
//!
//! The password column holds ciphertext from [`crate::crypto::LocalCipher`],
//! whose key is in the OS credential store. Reading the database file — a
//! backup, a synced folder, a disk pulled out of a laptop — yields nothing.
//!
//! **No method here returns a decrypted password.** [`LibraryDb::rcon_secret`]
//! is the only one that can, it is `pub(crate)`, and the only caller is the
//! connection path. There is deliberately no command above it: a webview that
//! could ask for a server password would make every other control here
//! decorative.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::crypto::LocalCipher;
use crate::error::{AppError, AppResult};
use crate::library::db::LibraryDb;
use crate::logging::now_rfc3339;

use super::RconProtocol;

/// Cap on saved servers.
pub const MAX_SERVERS: usize = 100;

/// A saved server, as the UI sees it.
///
/// Note what is absent: the password. It is not optional-and-omitted, it is not
/// a redacted placeholder — the type has no field for one, so no refactor can
/// accidentally start returning it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RconServer {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub protocol: RconProtocol,
    /// The TMC server this was created from, when it was.
    pub server_id: Option<i64>,
    pub app_id: Option<i64>,
    /// Whether a password is stored at all — so the UI can say "set a password"
    /// without being told what it is.
    pub has_password: bool,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

/// What creating one needs.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewRconServer {
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub protocol: RconProtocol,
    pub password: String,
    #[serde(default)]
    pub server_id: Option<i64>,
    #[serde(default)]
    pub app_id: Option<i64>,
}

impl LibraryDb {
    /// Save a server, encrypting its password.
    pub fn rcon_create(&self, cipher: &LocalCipher, new: &NewRconServer) -> AppResult<i64> {
        let name = new.name.trim();

        if name.is_empty() || name.len() > 96 {
            return Err(AppError::invalid("Give this server a name."));
        }

        if new.host.trim().is_empty() || new.host.len() > 253 {
            return Err(AppError::invalid("Enter the server's address."));
        }

        if new.port == 0 {
            return Err(AppError::invalid("Enter the RCON port."));
        }

        if self.rcon_count()? >= MAX_SERVERS {
            return Err(AppError::invalid(format!(
                "You already have {MAX_SERVERS} saved servers."
            )));
        }

        let sealed = if new.password.is_empty() {
            None
        } else {
            Some(cipher.seal(&new.password)?)
        };

        self.with(|conn| {
            conn.execute(
                r#"
                INSERT INTO rcon_server (
                    name, host, port, protocol, password, server_id, app_id, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                params![
                    name,
                    new.host.trim(),
                    new.port,
                    new.protocol.as_str(),
                    sealed,
                    new.server_id,
                    new.app_id,
                    now_rfc3339(),
                ],
            )?;

            Ok(conn.last_insert_rowid())
        })
    }

    pub fn rcon_count(&self) -> AppResult<usize> {
        self.with(|conn| {
            conn.query_row("SELECT COUNT(*) FROM rcon_server", [], |r| {
                r.get::<_, i64>(0)
            })
            .map(|n| n as usize)
        })
    }

    pub fn rcon_list(&self) -> AppResult<Vec<RconServer>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, host, port, protocol,
                        password IS NOT NULL, server_id, app_id, last_used_at, created_at
                 FROM rcon_server ORDER BY name ASC",
            )?;

            let rows: rusqlite::Result<Vec<RconServer>> =
                stmt.query_map([], row_to_server)?.collect();

            rows
        })
    }

    pub fn rcon_get(&self, id: i64) -> AppResult<Option<RconServer>> {
        self.with(|conn| {
            conn.prepare(
                "SELECT id, name, host, port, protocol,
                        password IS NOT NULL, server_id, app_id, last_used_at, created_at
                 FROM rcon_server WHERE id = ?1",
            )?
            .query_row(params![id], row_to_server)
            .optional()
        })
    }

    /// The decrypted password for one server.
    ///
    /// `pub(crate)` and called from exactly one place: the connection path. It
    /// is the only function in this crate that turns stored ciphertext back
    /// into a password, and there is no `#[tauri::command]` above it.
    pub(crate) fn rcon_secret(&self, cipher: &LocalCipher, id: i64) -> AppResult<String> {
        let sealed: Option<String> = self.with(|conn| {
            conn.query_row(
                "SELECT password FROM rcon_server WHERE id = ?1",
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map(Option::flatten)
        })?;

        let sealed = sealed.ok_or_else(|| {
            AppError::invalid("No password is saved for that server. Add one first.")
        })?;

        cipher.open(&sealed)
    }

    /// Change the stored password, or clear it with `None`.
    pub fn rcon_set_password(
        &self,
        cipher: &LocalCipher,
        id: i64,
        password: Option<&str>,
    ) -> AppResult<()> {
        let sealed = match password.filter(|p| !p.is_empty()) {
            Some(password) => Some(cipher.seal(password)?),
            None => None,
        };

        self.with(|conn| {
            conn.execute(
                "UPDATE rcon_server SET password = ?2 WHERE id = ?1",
                params![id, sealed],
            )?;

            Ok(())
        })
    }

    pub fn rcon_rename(&self, id: i64, name: &str, host: &str, port: u16) -> AppResult<()> {
        if name.trim().is_empty() || host.trim().is_empty() || port == 0 {
            return Err(AppError::invalid(
                "A saved server needs a name, an address and a port.",
            ));
        }

        self.with(|conn| {
            conn.execute(
                "UPDATE rcon_server SET name = ?2, host = ?3, port = ?4 WHERE id = ?1",
                params![id, name.trim(), host.trim(), port],
            )?;

            Ok(())
        })
    }

    pub fn rcon_delete(&self, id: i64) -> AppResult<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM rcon_server WHERE id = ?1", params![id])?;

            Ok(())
        })
    }

    pub fn rcon_touch(&self, id: i64) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE rcon_server SET last_used_at = ?2 WHERE id = ?1",
                params![id, now_rfc3339()],
            )?;

            Ok(())
        })
    }

    /// Append to the console history for one server.
    ///
    /// Bounded per server, and the OUTPUT is stored alongside the command —
    /// which is the point: a console that loses what a command printed the
    /// moment the pane closes is not a log, and "what did I run on this box
    /// last week" is most of why an admin wants one.
    pub fn rcon_log(&self, server_id: i64, command: &str, output: &str, ok: bool) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "INSERT INTO rcon_history (server_id, command, output, ok, at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    server_id,
                    truncate(command, 2048),
                    truncate(output, 64 * 1024),
                    ok as i64,
                    now_rfc3339()
                ],
            )?;

            // Keep the last 500 per server. An unbounded log is a database that
            // grows forever behind a feature nobody thinks of as storage.
            conn.execute(
                "DELETE FROM rcon_history
                 WHERE server_id = ?1 AND id NOT IN (
                     SELECT id FROM rcon_history WHERE server_id = ?1
                     ORDER BY id DESC LIMIT 500
                 )",
                params![server_id],
            )?;

            Ok(())
        })
    }

    pub fn rcon_history(&self, server_id: i64, limit: usize) -> AppResult<Vec<RconHistoryRow>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT command, output, ok, at FROM rcon_history
                 WHERE server_id = ?1 ORDER BY id DESC LIMIT ?2",
            )?;

            let rows: rusqlite::Result<Vec<RconHistoryRow>> = stmt
                .query_map(params![server_id, limit.min(500) as i64], |row| {
                    Ok(RconHistoryRow {
                        command: row.get(0)?,
                        output: row.get(1)?,
                        ok: row.get::<_, i64>(2)? != 0,
                        at: row.get(3)?,
                    })
                })?
                .collect();

            rows
        })
    }

    pub fn rcon_clear_history(&self, server_id: i64) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM rcon_history WHERE server_id = ?1",
                params![server_id],
            )?;

            Ok(())
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RconHistoryRow {
    pub command: String,
    pub output: String,
    pub ok: bool,
    pub at: String,
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }

    // On a character boundary, or the string is not valid UTF-8 when it comes
    // back out of SQLite.
    let mut end = max;

    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}…", &text[..end])
}

fn row_to_server(row: &rusqlite::Row<'_>) -> rusqlite::Result<RconServer> {
    Ok(RconServer {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        port: row.get::<_, i64>(3)? as u16,
        protocol: RconProtocol::parse(&row.get::<_, String>(4)?).unwrap_or_default(),
        has_password: row.get::<_, i64>(5)? != 0,
        server_id: row.get(6)?,
        app_id: row.get(7)?,
        last_used_at: row.get(8)?,
        created_at: row.get(9)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (LibraryDb, LocalCipher) {
        (
            LibraryDb::open_memory().expect("db"),
            LocalCipher::from_key(&[3u8; 32]),
        )
    }

    fn new_server(name: &str, password: &str) -> NewRconServer {
        NewRconServer {
            name: name.into(),
            host: "192.168.1.10".into(),
            port: 27015,
            protocol: RconProtocol::Source,
            password: password.into(),
            server_id: Some(42),
            app_id: Some(1),
        }
    }

    #[test]
    fn a_server_round_trips_without_its_password_ever_being_listed() {
        let (db, cipher) = setup();

        let id = db
            .rcon_create(&cipher, &new_server("My box", "hunter2"))
            .expect("create");

        let listed = db.rcon_list().expect("list");

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "My box");
        assert!(listed[0].has_password);

        // The password is reachable only through the one crate-private
        // accessor, and it round-trips.
        assert_eq!(db.rcon_secret(&cipher, id).expect("secret"), "hunter2");
    }

    /// The whole point of the encryption: somebody who copies the database file
    /// gets nothing.
    #[test]
    fn the_stored_password_is_not_the_password() {
        let (db, cipher) = setup();

        db.rcon_create(&cipher, &new_server("My box", "hunter2"))
            .expect("create");

        let stored: String = db
            .with(|conn| {
                conn.query_row("SELECT password FROM rcon_server", [], |r| {
                    r.get::<_, String>(0)
                })
            })
            .expect("read");

        assert!(!stored.contains("hunter2"));

        // And the key is what opens it — another key does not.
        let other = LocalCipher::from_key(&[9u8; 32]);

        assert!(other.open(&stored).is_err());
    }

    #[test]
    fn a_server_with_no_password_says_so_rather_than_storing_an_empty_one() {
        let (db, cipher) = setup();

        let id = db
            .rcon_create(&cipher, &new_server("No password yet", ""))
            .expect("create");

        assert!(!db.rcon_get(id).expect("get").expect("present").has_password);
        assert!(db.rcon_secret(&cipher, id).is_err());

        db.rcon_set_password(&cipher, id, Some("later"))
            .expect("set");

        assert!(db.rcon_get(id).expect("get").expect("present").has_password);
        assert_eq!(db.rcon_secret(&cipher, id).expect("secret"), "later");

        db.rcon_set_password(&cipher, id, None).expect("clear");

        assert!(!db.rcon_get(id).expect("get").expect("present").has_password);
    }

    #[test]
    fn a_server_needs_a_name_an_address_and_a_port() {
        let (db, cipher) = setup();

        let mut blank_name = new_server("", "x");
        blank_name.name = "   ".into();
        assert!(db.rcon_create(&cipher, &blank_name).is_err());

        let mut blank_host = new_server("A", "x");
        blank_host.host = String::new();
        assert!(db.rcon_create(&cipher, &blank_host).is_err());

        let mut no_port = new_server("A", "x");
        no_port.port = 0;
        assert!(db.rcon_create(&cipher, &no_port).is_err());
    }

    #[test]
    fn history_keeps_what_a_command_printed_and_stays_bounded() {
        let (db, cipher) = setup();

        let id = db
            .rcon_create(&cipher, &new_server("My box", "hunter2"))
            .expect("create");

        for n in 0..600 {
            db.rcon_log(id, &format!("say {n}"), "ok", true)
                .expect("log");
        }

        let rows = db.rcon_history(id, 500).expect("history");

        assert_eq!(rows.len(), 500, "the log must be bounded");
        // Newest first.
        assert_eq!(rows[0].command, "say 599");
        assert_eq!(rows[0].output, "ok");

        db.rcon_clear_history(id).expect("clear");

        assert!(db.rcon_history(id, 500).expect("history").is_empty());
    }

    #[test]
    fn deleting_a_server_takes_its_history_with_it() {
        let (db, cipher) = setup();

        let id = db
            .rcon_create(&cipher, &new_server("My box", "hunter2"))
            .expect("create");

        db.rcon_log(id, "status", "…", true).expect("log");
        db.rcon_delete(id).expect("delete");

        assert!(db.rcon_get(id).expect("get").is_none());
        assert!(db.rcon_history(id, 100).expect("history").is_empty());
    }

    #[test]
    fn a_very_long_output_is_truncated_on_a_character_boundary() {
        let (db, cipher) = setup();

        let id = db
            .rcon_create(&cipher, &new_server("My box", "hunter2"))
            .expect("create");

        // Multi-byte characters, so a naive byte cut would produce invalid
        // UTF-8 and fail on the way back out of SQLite.
        let long = "é".repeat(100_000);

        db.rcon_log(id, "status", &long, true).expect("log");

        let rows = db.rcon_history(id, 1).expect("history");

        assert!(rows[0].output.len() <= 64 * 1024 + 4);
        assert!(rows[0].output.ends_with('…'));
    }
}
