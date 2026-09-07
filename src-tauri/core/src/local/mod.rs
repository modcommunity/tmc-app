//! Content that is on this machine without an account behind it.
//!
//! The app's other content story is complete and narrow: an account subscribes,
//! the library mirrors the subscription, the installer materialises it, and
//! every one of those steps knows the item's id, its release and its checksum.
//! That is the right model for what the site holds. It is the whole model for
//! nothing else, and "nothing else" is most of what is on a modder's disk — the
//! jar from a Discord thread, the folder they built themselves, the two hundred
//! mods already sitting in Vortex, the pack they downloaded before they had an
//! account.
//!
//! | Module | Owns |
//! | --- | --- |
//! | [`metadata`] | `tmc.json` — what an archive says about where it came from |
//! | [`store`] | The files: importing, laying out, moving, removing |
//! | [`adopt`] | Finding what is already in a game folder that nothing claims |
//! | [`vault`] | How the webview names one of those without ever seeing a path |
//!
//! and this file owns the row: a name, a version, a note, and the one editable
//! field that matters, [`LocalMod::rel_path`].
//!
//! THE DESIGN IN ONE SENTENCE
//! --------------------------
//! A local mod is a directory of files plus editable text, and it is deployed
//! by exactly the code that deploys a subscribed one.
//!
//! [`crate::deploy::DeployMod`] wants a key, a name, a priority and a root
//! folder laid out as it should appear under the game directory. A staged
//! subscription is one of those. A local mod is another. So the merge tree, the
//! conflict report, the ledger, the backup-before-overwrite rule and the
//! "still ours" purge check all apply to an imported mod with no new code and,
//! more importantly, with no second set of rules to keep in step. A user
//! deploying a Vortex import next to a subscribed mod gets one conflict report
//! covering both, because there is only one.
//!
//! WHAT IS DELIBERATELY NOT HERE
//! -----------------------------
//! **Updates.** A local mod has no release history, no checksum from anybody
//! and no URL to re-fetch — that is what makes it local. Nothing here pretends
//! otherwise: [`LocalMod`] has no `latest_version`, the auto-updater does not
//! see one, and the honest way to update an import is to import the new file.
//! When a sidecar DOES name an item on this site, the row keeps the reference
//! ([`LocalMod::source`]) so the UI can offer "subscribe to this instead" — and
//! subscribing is what turns it into something the app can keep current.
//!
//! **A second identity space.** A local mod's `mod_key` is `local:<id>`, in the
//! same `sandbox_mod` table and the same ledger as `mod:1234`. One list, one
//! load order, one deploy.

pub mod adopt;
pub mod metadata;
pub mod store;
pub mod vault;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::library::db::LibraryDb;
use crate::logging::now_rfc3339;

pub use metadata::ModMetadata;
pub use store::{ImportOptions, Origin, Payload, Prepared};

/// The `kind` a local mod occupies in `sandbox_mod`, and the prefix of its key.
///
/// A reserved word rather than a content kind from the site's contract, and it
/// has to stay reserved: a future `ContentKind` called `local` would collide
/// with every imported mod's key.
pub const LOCAL_KIND: &str = "local";

/// Cap on how many local mods one device may hold.
///
/// Not a technical limit — it is the same reason a sandbox caps its mod count.
/// A runaway import loop should hit a sentence, not the disk.
pub const MAX_LOCAL_MODS: usize = 5_000;

/// Where a local mod claims to have come from on the site.
///
/// Only ever set from a [`ModMetadata`] whose `source` matched this build's API
/// base — see [`ModMetadata::item_ref`]. It is a LINK, never a claim of
/// ownership: nothing about this row is synced, updated or reported because of
/// it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub kind: String,
    pub item_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_url: Option<String>,
}

/// One imported mod, as the device holds it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalMod {
    pub id: i64,

    /// Editable. Defaults to the sidecar's name, then to the file or folder
    /// name — which is the answer the user asked for when they said "if we find
    /// a folder that isn't tied to anything, name it after the folder".
    pub name: String,

    /// The game. `None` is allowed and is not a broken row: an import made
    /// before a game was chosen is still a set of files, and the UI asks.
    pub app_id: Option<i64>,
    pub app_slug: Option<String>,

    pub version: Option<String>,
    pub author: Option<String>,
    /// The user's own note. Never shown to anybody else — nothing here syncs.
    pub notes: Option<String>,

    pub origin: Origin,
    /// Where it came from in words: the dropped file's name, the folder it was
    /// adopted from, "Vortex — Skyrim Special Edition".
    pub origin_label: Option<String>,

    /// Where its files sit under the game folder. Editable, and changing it
    /// moves the files — see [`store::relayout`].
    pub rel_path: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRef>,

    pub files: i64,
    pub bytes: i64,

    pub added_at: String,
    pub updated_at: String,
}

impl LocalMod {
    /// The key this mod occupies in a sandbox's load order and in the ledger.
    pub fn mod_key(&self) -> String {
        format!("{LOCAL_KIND}:{}", self.id)
    }

    /// Is `key` a local mod's, and which one?
    pub fn id_from_key(key: &str) -> Option<i64> {
        key.strip_prefix("local:")?.parse().ok()
    }
}

/// The fields a caller may change after an import.
///
/// Every one of them is text the user typed about their own files, so there is
/// nothing here to validate beyond length — except `rel_path`, which is a path
/// and goes through the same check every other path in this crate does.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPatch {
    pub name: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub notes: Option<String>,
    pub app_id: Option<i64>,
    pub app_slug: Option<String>,
    /// Moving the files is the caller's job — [`store::relayout`] — because
    /// only it knows the store's root. This records the result.
    pub rel_path: Option<String>,
}

/// What a new row starts as.
#[derive(Debug, Clone)]
pub struct NewLocalMod {
    pub name: String,
    pub app_id: Option<i64>,
    pub app_slug: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub origin: Origin,
    pub origin_label: Option<String>,
    pub rel_path: String,
    pub source: Option<SourceRef>,
    pub files: i64,
    pub bytes: i64,
}

impl LibraryDb {
    /// Every local mod, or one game's.
    pub fn local_list(&self, app_id: Option<i64>) -> AppResult<Vec<LocalMod>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, app_id, app_slug, version, author, notes,
                        origin, origin_label, rel_path,
                        source_kind, source_item, source_release, source_url,
                        files, bytes, added_at, updated_at
                 FROM local_mod
                 WHERE (?1 IS NULL OR app_id = ?1)
                 ORDER BY name COLLATE NOCASE ASC, id ASC",
            )?;

            let rows: rusqlite::Result<Vec<LocalMod>> =
                stmt.query_map(params![app_id], read_row)?.collect();

            rows
        })
    }

    pub fn local_get(&self, id: i64) -> AppResult<Option<LocalMod>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT id, name, app_id, app_slug, version, author, notes,
                        origin, origin_label, rel_path,
                        source_kind, source_item, source_release, source_url,
                        files, bytes, added_at, updated_at
                 FROM local_mod WHERE id = ?1",
                params![id],
                read_row,
            )
            .optional()
        })
    }

    /// Insert a row and hand back its id, so the caller can move the files into
    /// `<store>/<id>`.
    ///
    /// The row lands BEFORE the files do, deliberately: a crash between the two
    /// leaves a row with no directory, which every reader already handles (the
    /// mod deploys nothing and says so), while the other order leaves a
    /// directory nothing owns and nothing ever cleans up.
    pub fn local_create(&self, new: &NewLocalMod) -> AppResult<i64> {
        let count = self.local_count()?;

        if count >= MAX_LOCAL_MODS {
            return Err(AppError::invalid(format!(
                "This device already holds {MAX_LOCAL_MODS} imported mods."
            )));
        }

        let now = now_rfc3339();

        self.with(|conn| {
            conn.execute(
                r#"
                INSERT INTO local_mod (
                    name, app_id, app_slug, version, author, notes,
                    origin, origin_label, rel_path,
                    source_kind, source_item, source_release, source_url,
                    files, bytes, added_at, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, NULL,
                    ?6, ?7, ?8,
                    ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?15
                )
                "#,
                params![
                    new.name,
                    new.app_id,
                    new.app_slug,
                    new.version,
                    new.author,
                    new.origin.as_str(),
                    new.origin_label,
                    new.rel_path,
                    new.source.as_ref().map(|s| s.kind.clone()),
                    new.source.as_ref().map(|s| s.item_id),
                    new.source.as_ref().and_then(|s| s.release_id),
                    new.source.as_ref().and_then(|s| s.web_url.clone()),
                    new.files,
                    new.bytes,
                    now,
                ],
            )?;

            Ok(conn.last_insert_rowid())
        })
    }

    pub fn local_count(&self) -> AppResult<usize> {
        self.with(|conn| {
            conn.query_row("SELECT COUNT(*) FROM local_mod", [], |r| r.get::<_, i64>(0))
                .map(|n| n as usize)
        })
    }

    /// Apply the editable fields.
    ///
    /// `name` is clamped to something non-empty rather than refused: a user
    /// clearing the box wants the old name back, not an error dialog.
    pub fn local_patch(&self, id: i64, patch: &LocalPatch) -> AppResult<LocalMod> {
        let Some(current) = self.local_get(id)? else {
            return Err(AppError::invalid(
                "That imported mod is not on this device.",
            ));
        };

        let name = patch
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| n.chars().take(200).collect::<String>())
            .unwrap_or(current.name);

        fn text(next: Option<&String>, current: Option<String>, max: usize) -> Option<String> {
            match next {
                Some(raw) => {
                    let cleaned: String = raw.trim().chars().take(max).collect();

                    if cleaned.is_empty() {
                        None
                    } else {
                        Some(cleaned)
                    }
                }
                None => current,
            }
        }

        let version = text(patch.version.as_ref(), current.version, 64);
        let author = text(patch.author.as_ref(), current.author, 120);
        let notes = text(patch.notes.as_ref(), current.notes, 4000);

        let rel_path = match &patch.rel_path {
            Some(raw) => store::sanitise_rel(raw)?,
            None => current.rel_path,
        };

        let app_id = patch.app_id.or(current.app_id);
        let app_slug = patch.app_slug.clone().or(current.app_slug);

        self.with(|conn| {
            conn.execute(
                "UPDATE local_mod
                 SET name = ?2, version = ?3, author = ?4, notes = ?5,
                     app_id = ?6, app_slug = ?7, rel_path = ?8, updated_at = ?9
                 WHERE id = ?1",
                params![
                    id,
                    name,
                    version,
                    author,
                    notes,
                    app_id,
                    app_slug,
                    rel_path,
                    now_rfc3339()
                ],
            )?;

            Ok(())
        })?;

        self.local_get(id)?
            .ok_or_else(|| AppError::internal("the imported mod vanished"))
    }

    /// Record what is actually on disk, after an import or a re-measure.
    pub fn local_set_size(&self, id: i64, files: i64, bytes: i64) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE local_mod SET files = ?2, bytes = ?3 WHERE id = ?1",
                params![id, files, bytes],
            )?;

            Ok(())
        })
    }

    /// Forget a local mod, and take it out of every sandbox holding it.
    ///
    /// Both halves, in one call, because a sandbox row pointing at a deleted
    /// local mod is a member with no files that reports no error — it appears
    /// in the load order, appears enabled, and silently contributes nothing.
    /// Deleting the files is the CALLER's job (it holds the store root), and
    /// this returns the sandboxes that have to be re-deployed.
    pub fn local_delete(&self, id: i64) -> AppResult<Vec<i64>> {
        let affected = self.local_sandboxes(id)?;

        let key = format!("{LOCAL_KIND}:{id}");

        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;

            tx.execute("DELETE FROM sandbox_mod WHERE mod_key = ?1", params![key])?;

            tx.execute("DELETE FROM local_mod WHERE id = ?1", params![id])?;

            tx.commit()?;

            Ok(())
        })?;

        Ok(affected)
    }

    /// Which sandboxes hold this local mod.
    pub fn local_sandboxes(&self, id: i64) -> AppResult<Vec<i64>> {
        let key = format!("{LOCAL_KIND}:{id}");

        self.with(|conn| {
            let mut stmt =
                conn.prepare("SELECT sandbox_id FROM sandbox_mod WHERE mod_key = ?1 ORDER BY 1")?;

            let rows: rusqlite::Result<Vec<i64>> =
                stmt.query_map(params![key], |r| r.get(0))?.collect();

            rows
        })
    }

    /// Put a local mod in a sandbox.
    ///
    /// Marked staged on the spot, and that is the one place a local mod's
    /// lifecycle differs from a subscription's: staging a subscription is a
    /// download, and an import HAS ALREADY HAPPENED — its files are in the
    /// store. Leaving `staged_at` null instead would make the mod invisible to
    /// [`crate::library::deploy::deployable`], which is the correct filter for
    /// a mod that has not downloaded and exactly wrong for one that never will.
    pub fn sandbox_add_local(&self, sandbox_id: i64, local: &LocalMod) -> AppResult<String> {
        let key = self.sandbox_add_mod(sandbox_id, LOCAL_KIND, local.id, &local.name)?;

        self.sandbox_mark_staged(sandbox_id, &key, None, local.version.as_deref())?;

        Ok(key)
    }

    /// Every path an ADOPTED local mod occupies in the game folder.
    ///
    /// What makes a second adopt scan idempotent — without it the same folder
    /// is offered again on every run, and adopting it twice gives one mod two
    /// rows whose files conflict with each other at every path.
    ///
    /// Read from the STORE rather than from the row, and that is the whole
    /// subtlety: a row's `rel_path` is where its payload is *rooted*
    /// (`mods`), while the scan lists *entries* (`mods/BigMod`). Comparing the
    /// two directly never matches. So each adopted mod's top level is
    /// enumerated under its own root and the two are compared as the same kind
    /// of thing.
    pub fn local_claimed_paths(
        &self,
        app_id: Option<i64>,
        store: &std::path::Path,
    ) -> AppResult<Vec<String>> {
        let rows: Vec<(i64, String)> = self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, rel_path FROM local_mod
                 WHERE origin = 'adopted' AND (?1 IS NULL OR app_id = ?1)",
            )?;

            let rows: rusqlite::Result<Vec<(i64, String)>> = stmt
                .query_map(params![app_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect();

            rows
        })?;

        let mut out = Vec::new();

        for (id, rel_path) in rows {
            let mut root = store::local_root(store, id);

            if !rel_path.is_empty() {
                match crate::plugins::jail::join_relative(&root, &rel_path) {
                    Ok(path) => root = path,
                    Err(_) => continue,
                }
            }

            let Ok(entries) = std::fs::read_dir(&root) else {
                continue;
            };

            for entry in entries.flatten().take(1_000) {
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };

                out.push(if rel_path.is_empty() {
                    name
                } else {
                    format!("{rel_path}/{name}")
                });
            }
        }

        Ok(out)
    }
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalMod> {
    let source_kind: Option<String> = row.get(10)?;
    let source_item: Option<i64> = row.get(11)?;

    let source = match (source_kind, source_item) {
        (Some(kind), Some(item_id)) => Some(SourceRef {
            kind,
            item_id,
            release_id: row.get(12)?,
            web_url: row.get(13)?,
        }),
        _ => None,
    };

    Ok(LocalMod {
        id: row.get(0)?,
        name: row.get(1)?,
        app_id: row.get(2)?,
        app_slug: row.get(3)?,
        version: row.get(4)?,
        author: row.get(5)?,
        notes: row.get(6)?,
        origin: Origin::parse(&row.get::<_, String>(7)?),
        origin_label: row.get(8)?,
        rel_path: row.get(9)?,
        source,
        files: row.get(14)?,
        bytes: row.get(15)?,
        added_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::sandbox::{Environment, NewSandbox};

    fn db() -> LibraryDb {
        LibraryDb::open_memory().expect("db")
    }

    fn sandbox(db: &LibraryDb) -> i64 {
        db.sandbox_create(&new_sandbox()).expect("sandbox")
    }

    fn new_sandbox() -> NewSandbox {
        NewSandbox {
            app_id: 5,
            app_slug: Some("minecraft".into()),
            app_name: None,
            name: "Test".into(),
            description: None,
            environment: Environment::Client,
            strategy: Default::default(),
            game_version: None,
            loader: None,
            preset: None,
            game_dir: None,
            options: Default::default(),
            cloud_sync: false,
            auto_update: false,
        }
    }

    fn new_local(name: &str) -> NewLocalMod {
        NewLocalMod {
            name: name.to_string(),
            app_id: Some(5),
            app_slug: Some("minecraft".into()),
            version: Some("1.0".into()),
            author: None,
            origin: Origin::Dropped,
            origin_label: Some("thing.jar".into()),
            rel_path: "mods".into(),
            source: None,
            files: 1,
            bytes: 100,
        }
    }

    #[test]
    fn a_local_mod_round_trips() {
        let db = db();
        let id = db.local_create(&new_local("Cool Mod")).expect("create");

        let row = db.local_get(id).expect("get").expect("some");

        assert_eq!(row.name, "Cool Mod");
        assert_eq!(row.rel_path, "mods");
        assert_eq!(row.origin, Origin::Dropped);
        assert_eq!(row.mod_key(), format!("local:{id}"));
        assert_eq!(LocalMod::id_from_key(&row.mod_key()), Some(id));
    }

    #[test]
    fn the_editable_fields_are_editable_and_the_rest_are_not_touched() {
        let db = db();
        let id = db.local_create(&new_local("Original")).expect("create");

        let patched = db
            .local_patch(
                id,
                &LocalPatch {
                    name: Some("Renamed".into()),
                    notes: Some("  my note  ".into()),
                    ..Default::default()
                },
            )
            .expect("patch");

        assert_eq!(patched.name, "Renamed");
        assert_eq!(patched.notes.as_deref(), Some("my note"));
        // Untouched by a patch that did not name them.
        assert_eq!(patched.version.as_deref(), Some("1.0"));
        assert_eq!(patched.rel_path, "mods");
    }

    #[test]
    fn clearing_the_name_keeps_the_old_one_rather_than_erroring() {
        let db = db();
        let id = db.local_create(&new_local("Original")).expect("create");

        let patched = db
            .local_patch(
                id,
                &LocalPatch {
                    name: Some("   ".into()),
                    ..Default::default()
                },
            )
            .expect("patch");

        assert_eq!(patched.name, "Original");
    }

    #[test]
    fn an_install_path_that_escapes_is_refused_by_the_patch() {
        let db = db();
        let id = db.local_create(&new_local("Original")).expect("create");

        assert!(db
            .local_patch(
                id,
                &LocalPatch {
                    rel_path: Some("../../etc".into()),
                    ..Default::default()
                }
            )
            .is_err());
    }

    /// The whole reason `local_delete` does both halves: a sandbox row pointing
    /// at a deleted local mod deploys nothing and reports nothing.
    #[test]
    fn deleting_a_local_mod_takes_it_out_of_every_sandbox() {
        let db = db();

        let sandbox_id = sandbox(&db);

        let id = db.local_create(&new_local("Cool Mod")).expect("create");
        let row = db.local_get(id).expect("get").expect("some");

        let key = db.sandbox_add_local(sandbox_id, &row).expect("add");

        let members = db.sandbox_mods(sandbox_id).expect("mods");

        assert_eq!(members.len(), 1);
        assert_eq!(members[0].mod_key, key);
        // Staged on the spot — the files are already in the store.
        assert!(members[0].staged_at.is_some());

        assert_eq!(db.local_sandboxes(id).expect("used by"), vec![sandbox_id]);

        let affected = db.local_delete(id).expect("delete");

        assert_eq!(affected, vec![sandbox_id]);
        assert!(db.sandbox_mods(sandbox_id).expect("mods").is_empty());
        assert!(db.local_get(id).expect("get").is_none());
    }
}
