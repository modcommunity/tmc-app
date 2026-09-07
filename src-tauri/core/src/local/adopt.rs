//! Finding what is already in a game folder that nothing here put there.
//!
//! Somebody who installs this app has been modding for years. Their
//! `Valheim/BepInEx/plugins` has thirty DLLs in it, their `mods` folder has
//! ninety jars, and not one of them arrived through a subscription. A manager
//! that can only see what it installed itself shows that person an empty list
//! and asks them to start again — which is the moment most people stop
//! evaluating a mod manager.
//!
//! So: list what is in the folders the game's own rules say mods live in,
//! subtract everything the app can account for, and offer the rest.
//!
//! WHAT "ACCOUNT FOR" MEANS, AND WHY IT HAS TO BE EXACT
//! ---------------------------------------------------
//! Three claims, all of them records this app already keeps:
//!
//!   * the **deployment ledger** — every file any sandbox put in this folder;
//!   * a **subscription's installed files** — what the main-install path wrote;
//!   * an **already-adopted local mod** — so a second scan does not offer the
//!     same folder twice.
//!
//! Getting the subtraction wrong is worse than not offering the feature. Adopt
//! something the app already deploys and the user has one mod listed twice,
//! with the merge tree correctly reporting every one of its files as a conflict
//! with itself. That is why this reads the ledger rather than guessing from
//! file dates or from a marker file: the ledger exists precisely because a
//! rescan cannot tell whose file this is, and the same argument applies here.
//!
//! WHAT THIS IS NOT
//! ----------------
//! It is not a filesystem scan. It reads the immediate children of directories
//! a game's `sandbox.json` declares, and nothing else — no walk, no depth, no
//! `~/Documents`. A game with no `sandbox.json` therefore offers nothing to
//! adopt, which is the honest answer rather than a guess about where its mods
//! might be.
//!
//! It also **moves nothing**. A candidate is a report; importing one COPIES it
//! into the store and leaves the original exactly where it is. The user's game
//! keeps working whether or not they finish the import, and undoing an adoption
//! is deleting a row.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde::Serialize;

use crate::error::AppResult;
use crate::library::db::LibraryDb;

/// Cap on the entries one scan reports.
///
/// A folder with more than this in it is not a list of mods, it is a game's own
/// asset directory that somebody pointed a mod target at — and rendering four
/// thousand checkboxes helps nobody.
pub const MAX_CANDIDATES: usize = 500;

/// Something in a game folder that the app cannot account for.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    /// What it would be called if imported: the folder or file name. Editable
    /// afterwards, which is the whole answer to "name it the folder name and
    /// let the user change it".
    pub name: String,

    /// Where it sits under the game folder, `/`-separated. This becomes the
    /// imported mod's install path, so an adopted `BepInEx/plugins/Thing.dll`
    /// deploys back to exactly where it was found.
    pub rel_path: String,

    /// A directory of files, or one loose file.
    pub is_dir: bool,

    pub files: usize,
    pub bytes: u64,

    /// The absolute path, for the command layer to mint a token from.
    ///
    /// `skip`ped on the wire: the webview names candidates by token, never by
    /// path, exactly as it does for a dropped file. See `commands::import`.
    #[serde(skip)]
    pub path: PathBuf,
}

/// What a scan needs to know.
pub struct AdoptScan<'a> {
    /// The game folder itself.
    pub game_dir: &'a Path,
    /// Where this game's `sandbox.json` says mods live, `/`-separated and
    /// relative to `game_dir`.
    pub mod_targets: &'a [String],
    /// The game, so the claims are looked up for the right one.
    pub app_id: Option<i64>,
    /// The device's imported-mod store, so a folder adopted on a previous run
    /// is not offered again. See [`LibraryDb::local_claimed_paths`].
    pub local_store: &'a Path,
}

/// Everything in the game's mod folders that nothing here claims.
pub fn scan(db: &LibraryDb, request: &AdoptScan<'_>) -> AppResult<Vec<Candidate>> {
    let claimed = claims(db, request)?;

    let mut out: Vec<Candidate> = Vec::new();

    for target in request.mod_targets {
        if out.len() >= MAX_CANDIDATES {
            break;
        }

        let clean = match super::store::sanitise_rel(target) {
            Ok(clean) => clean,
            Err(_) => continue,
        };

        let dir = if clean.is_empty() {
            request.game_dir.to_path_buf()
        } else {
            match crate::plugins::jail::join_relative(request.game_dir, &clean) {
                Ok(path) => path,
                Err(_) => continue,
            }
        };

        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            if out.len() >= MAX_CANDIDATES {
                break;
            }

            // A symlink in a game folder is somebody else's deployment — MO2
            // and r2modman both work this way — and following it would import
            // the file it points at while leaving the link behind.
            if entry.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }

            let Ok(meta) = entry.metadata() else { continue };

            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };

            if is_noise(&name) {
                continue;
            }

            let rel = if clean.is_empty() {
                name.clone()
            } else {
                format!("{clean}/{name}")
            };

            if claimed.contains(&rel.to_ascii_lowercase()) {
                continue;
            }

            // A directory whose files the app deployed is claimed even though
            // the directory itself is not in the ledger — the ledger names
            // files, and a mod that IS a folder puts its files under one.
            if meta.is_dir() && claimed_under(&claimed, &rel) {
                continue;
            }

            let (files, bytes) = if meta.is_dir() {
                measure(&entry.path())
            } else if meta.is_file() {
                (1, meta.len())
            } else {
                continue;
            };

            if files == 0 {
                continue;
            }

            out.push(Candidate {
                name: display_name(&name, meta.is_dir()),
                rel_path: rel,
                is_dir: meta.is_dir(),
                files,
                bytes,
                path: entry.path(),
            });
        }
    }

    out.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.rel_path.cmp(&b.rel_path))
    });

    Ok(out)
}

/// Every path in this game's folder that the app can already account for,
/// lower-cased so a case-insensitive filesystem does not defeat the check.
fn claims(db: &LibraryDb, request: &AdoptScan<'_>) -> AppResult<BTreeSet<String>> {
    let mut out = BTreeSet::new();

    // 1. Every file any sandbox for this game has deployed.
    let deployed: Vec<String> = db.with(|conn| {
        let mut stmt = conn.prepare(
            "SELECT d.path FROM deployment d
             JOIN sandbox s ON s.id = d.sandbox_id
             WHERE (?1 IS NULL OR s.app_id = ?1)",
        )?;

        let rows: rusqlite::Result<Vec<String>> = stmt
            .query_map(params![request.app_id], |r| r.get(0))?
            .collect();

        rows
    })?;

    for path in deployed {
        out.insert(normalise(&path));
    }

    // 2. Every file the main-install path wrote for a subscription.
    for entry in db.list()? {
        if request.app_id.is_some() && entry.app_id != request.app_id {
            continue;
        }

        for path in entry.installed_files {
            out.insert(normalise(&path));
        }
    }

    // 3. Anything already adopted, so a second scan is idempotent.
    for path in db.local_claimed_paths(request.app_id, request.local_store)? {
        out.insert(normalise(&path));
    }

    Ok(out)
}

/// Does anything claimed live inside this directory?
fn claimed_under(claimed: &BTreeSet<String>, rel: &str) -> bool {
    let prefix = format!("{}/", rel.to_ascii_lowercase());

    claimed
        .range(prefix.clone()..)
        .next()
        .is_some_and(|first| first.starts_with(&prefix))
}

/// `/`-separated, lower-case, no leading slash — the one shape a claim is
/// compared in. The ledger stores this shape already; an installer's recorded
/// path may be absolute or native-separated.
fn normalise(path: &str) -> String {
    let unified = path.replace('\\', "/");

    unified
        .trim_start_matches('/')
        .to_ascii_lowercase()
        .to_string()
}

/// Things that are in every mod folder and are never a mod.
fn is_noise(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();

    lower == ".ds_store"
        || lower == "thumbs.db"
        || lower == "desktop.ini"
        || lower.ends_with(".tmcpart")
        || lower.ends_with(".bak")
        || lower.ends_with(".log")
}

/// Strip a file's extension for the suggested name; leave a folder's alone.
fn display_name(name: &str, is_dir: bool) -> String {
    if is_dir {
        return name.to_string();
    }

    name.rsplit_once('.')
        .map(|(head, _)| head)
        .filter(|head| !head.is_empty())
        .unwrap_or(name)
        .to_string()
}

/// Count a candidate directory, bounded — a mod folder is small and a game's
/// asset tree is not, and this runs once per entry in a listing.
fn measure(dir: &Path) -> (usize, u64) {
    fn inner(dir: &Path, depth: usize, files: &mut usize, bytes: &mut u64) {
        if depth > super::store::MAX_IMPORT_DEPTH || *files > super::store::MAX_IMPORT_FILES {
            return;
        }

        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }

            let Ok(meta) = entry.metadata() else { continue };

            if meta.is_dir() {
                inner(&entry.path(), depth + 1, files, bytes);
            } else if meta.is_file() {
                *files += 1;
                *bytes += meta.len();
            }
        }
    }

    let mut files = 0usize;
    let mut bytes = 0u64;

    inner(dir, 0, &mut files, &mut bytes);

    (files, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::LedgerEntry;
    use crate::library::sandbox::{Environment, NewSandbox};

    fn new_sandbox() -> NewSandbox {
        NewSandbox {
            app_id: 1,
            app_slug: None,
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

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }

        std::fs::write(path, body).expect("write");
    }

    #[test]
    fn an_unmanaged_folder_and_file_are_both_offered() {
        let db = LibraryDb::open_memory().expect("db");
        let game = tempfile::tempdir().expect("tempdir");
        let store = tempfile::tempdir().expect("tempdir");

        write(&game.path().join("mods/AlreadyThere.jar"), "x");
        write(&game.path().join("mods/BigMod/main.dll"), "y");
        write(&game.path().join("mods/BigMod/data/thing.bin"), "zz");

        let targets = vec!["mods".to_string()];

        let found = scan(
            &db,
            &AdoptScan {
                game_dir: game.path(),
                mod_targets: &targets,
                app_id: Some(1),
                local_store: store.path(),
            },
        )
        .expect("scan");

        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();

        assert_eq!(names, vec!["AlreadyThere", "BigMod"]);

        let folder = found.iter().find(|c| c.is_dir).expect("dir");

        assert_eq!(folder.rel_path, "mods/BigMod");
        assert_eq!(folder.files, 2);
    }

    /// The failure this scan exists to avoid: offering to adopt a mod the app
    /// itself deployed, which lists it twice and makes every file conflict
    /// with itself.
    #[test]
    fn nothing_the_app_deployed_is_offered() {
        let db = LibraryDb::open_memory().expect("db");
        let game = tempfile::tempdir().expect("tempdir");
        let store = tempfile::tempdir().expect("tempdir");

        write(&game.path().join("mods/Ours.jar"), "x");
        write(&game.path().join("mods/Theirs.jar"), "y");
        write(&game.path().join("mods/OurFolder/inner.dll"), "z");

        let sandbox_id = db.sandbox_create(&new_sandbox()).expect("sandbox");

        db.sandbox_set_ledger(
            sandbox_id,
            &[
                LedgerEntry {
                    path: "mods/Ours.jar".into(),
                    kind: "copy".into(),
                    mod_key: "mod:1".into(),
                    source: "/staging/mods/Ours.jar".into(),
                    size: 1,
                    mtime_ms: 0,
                    backup: None,
                },
                LedgerEntry {
                    path: "mods/OurFolder/inner.dll".into(),
                    kind: "copy".into(),
                    mod_key: "mod:2".into(),
                    source: "/staging/mods/OurFolder/inner.dll".into(),
                    size: 1,
                    mtime_ms: 0,
                    backup: None,
                },
            ],
        )
        .expect("ledger");

        let targets = vec!["mods".to_string()];

        let found = scan(
            &db,
            &AdoptScan {
                game_dir: game.path(),
                mod_targets: &targets,
                app_id: Some(1),
                local_store: store.path(),
            },
        )
        .expect("scan");

        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();

        // `OurFolder` is claimed by the file INSIDE it, which is the only way
        // the ledger can express a mod that is a directory.
        assert_eq!(names, vec!["Theirs"]);
    }

    /// Scanning twice must not offer the same folder twice.
    ///
    /// The subtlety this pins: an adopted mod's ROW records where its payload
    /// is rooted (`mods`), while the scan lists ENTRIES (`mods/BigMod`).
    /// Comparing those directly never matches, so the same folder comes back on
    /// every run and adopting it again gives one mod two rows whose files
    /// conflict with each other at every path.
    #[test]
    fn a_folder_already_adopted_is_not_offered_again() {
        use crate::local::{store, NewLocalMod, Origin};

        let db = LibraryDb::open_memory().expect("db");
        let game = tempfile::tempdir().expect("tempdir");
        let store_dir = tempfile::tempdir().expect("tempdir");

        write(&game.path().join("mods/BigMod/main.dll"), "x");
        write(&game.path().join("mods/Other.jar"), "y");

        let targets = vec!["mods".to_string()];

        let request = AdoptScan {
            game_dir: game.path(),
            mod_targets: &targets,
            app_id: Some(1),
            local_store: store_dir.path(),
        };

        let first = scan(&db, &request).expect("scan");

        assert_eq!(first.len(), 2);

        // Adopt one of them, exactly as the command layer does: the payload is
        // rooted at the target folder and the entry itself is copied under it.
        let id = db
            .local_create(&NewLocalMod {
                name: "BigMod".into(),
                app_id: Some(1),
                app_slug: None,
                version: None,
                author: None,
                origin: Origin::Adopted,
                origin_label: Some("BigMod".into()),
                rel_path: "mods".into(),
                source: None,
                files: 1,
                bytes: 1,
            })
            .expect("create");

        write(
            &store::local_root(store_dir.path(), id).join("mods/BigMod/main.dll"),
            "x",
        );

        let second = scan(&db, &request).expect("rescan");

        let names: Vec<&str> = second.iter().map(|c| c.name.as_str()).collect();

        assert_eq!(names, vec!["Other"]);
    }

    #[test]
    fn a_game_with_no_mod_targets_offers_nothing() {
        let db = LibraryDb::open_memory().expect("db");
        let game = tempfile::tempdir().expect("tempdir");
        let store = tempfile::tempdir().expect("tempdir");

        write(&game.path().join("mods/Thing.jar"), "x");

        let found = scan(
            &db,
            &AdoptScan {
                game_dir: game.path(),
                mod_targets: &[],
                app_id: None,
                local_store: store.path(),
            },
        )
        .expect("scan");

        assert!(found.is_empty());
    }

    #[test]
    fn noise_is_not_a_mod() {
        let db = LibraryDb::open_memory().expect("db");
        let game = tempfile::tempdir().expect("tempdir");
        let store = tempfile::tempdir().expect("tempdir");

        write(&game.path().join("mods/.DS_Store"), "x");
        write(&game.path().join("mods/install.log"), "x");
        write(&game.path().join("mods/Real.jar"), "x");

        let targets = vec!["mods".to_string()];

        let found = scan(
            &db,
            &AdoptScan {
                game_dir: game.path(),
                mod_targets: &targets,
                app_id: None,
                local_store: store.path(),
            },
        )
        .expect("scan");

        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();

        assert_eq!(names, vec!["Real"]);
    }
}
