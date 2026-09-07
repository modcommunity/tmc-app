//! Finding games GOG Galaxy has installed, and GOG games installed without it.
//!
//! Galaxy keeps its state in a SQLite database, which is convenient — this
//! crate already links SQLite for the library — and delicate, because Galaxy
//! may be running and holding it open.
//!
//! ```text
//! C:\ProgramData\GOG.com\Galaxy\storage\galaxy-2.0.db
//! ~/Library/Application Support/GOG.com/Galaxy/storage/galaxy-2.0.db
//! ```
//!
//! **Opened read-only and `immutable=1`.** Read-only alone is not enough: it
//! still takes a shared lock and still reads the WAL, so a Galaxy mid-write
//! either blocks this or hands it a torn read. `immutable` tells SQLite the
//! file cannot change underneath it, which skips locking entirely. The cost is
//! that a database Galaxy is actively writing may read slightly stale — which,
//! for "where is this game installed", is a difference nobody can observe.
//!
//! This app never writes to it, and a failure to open one is not an error: a
//! machine with no Galaxy is the normal case on four of the five platforms.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use super::{DetectRoots, DetectedGame};

/// Cap on rows read. A large GOG library is a few hundred games.
const MAX_ROWS: usize = 5_000;

/// Where Galaxy's database lives on this machine.
fn databases(roots: &DetectRoots) -> Vec<PathBuf> {
    let mut out = Vec::new();

    const REL: &str = "GOG.com/Galaxy/storage/galaxy-2.0.db";

    if let Some(program_data) = &roots.program_data {
        out.push(program_data.join(REL));
    }

    if let Some(home) = &roots.home {
        out.push(home.join("Library/Application Support").join(REL));
    }

    for drive in &roots.drives {
        out.push(drive.join("ProgramData").join(REL));
    }

    out.retain(|path| path.is_file());

    out
}

pub fn scan(roots: &DetectRoots) -> Vec<DetectedGame> {
    let mut out = Vec::new();

    for db in databases(roots) {
        read_galaxy(&db, &mut out);
    }

    scan_offline(roots, &mut out);

    out
}

fn read_galaxy(path: &Path, out: &mut Vec<DetectedGame>) {
    // `immutable=1` — see the module header. The URI has to be percent-safe,
    // and a path with a `?` in it would otherwise become query parameters.
    let uri = format!(
        "file:{}?mode=ro&immutable=1",
        path.to_string_lossy()
            .replace('?', "%3f")
            .replace('#', "%23")
    );

    let Ok(conn) = Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    ) else {
        return;
    };

    /*
     * `LEFT JOIN` on the title: `InstalledBaseProducts` is the authority on
     * what is installed, and `LimitedDetails` is a metadata cache that a fresh
     * install or an offline Galaxy may not have filled in. Requiring the title
     * would mean finding no games on exactly the machines where Galaxy has not
     * been online.
     */
    let sql = "SELECT p.productId, p.installationPath, d.title
               FROM InstalledBaseProducts p
               LEFT JOIN LimitedDetails d ON d.productId = p.productId";

    let Ok(mut stmt) = conn.prepare(sql) else {
        return;
    };

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    });

    let Ok(rows) = rows else {
        return;
    };

    for row in rows.flatten().take(MAX_ROWS) {
        let (product_id, install_path, title) = row;

        let dir = PathBuf::from(&install_path);

        if !dir.is_dir() {
            continue;
        }

        let name = title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| folder_name(&dir));

        out.push(DetectedGame {
            source: "gog".into(),
            name: name.trim().to_string(),
            path: install_path,
            launcher_id: Some(product_id.to_string()),
            launch_uri: None,
            size_bytes: None,
            slug: None,
        });
    }
}

/// GOG's installers do not need Galaxy, and a lot of people never install it.
/// Those games land in one predictable folder.
fn scan_offline(roots: &DetectRoots, out: &mut Vec<DetectedGame>) {
    let mut candidates: Vec<PathBuf> = Vec::new();

    for drive in &roots.drives {
        candidates.push(drive.join("GOG Games"));
        candidates.push(drive.join("Games/GOG"));
    }

    for base in &roots.program_files {
        candidates.push(base.join("GOG Galaxy/Games"));
    }

    if let Some(home) = &roots.home {
        candidates.push(home.join("GOG Games"));
        // The usual Linux/Proton layout for GOG's own installers.
        candidates.push(home.join("Games"));
    }

    for dir in candidates {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten().take(MAX_ROWS) {
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            // Already found through Galaxy — the database is the better source,
            // since it carries the product id.
            if out.iter().any(|g| Path::new(&g.path) == path) {
                continue;
            }

            out.push(DetectedGame {
                source: "gog".into(),
                name: folder_name(&path),
                path: path.to_string_lossy().into_owned(),
                launcher_id: None,
                launch_uri: None,
                size_bytes: None,
                slug: None,
            });
        }
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn galaxy_db(at: &Path, rows: &[(i64, &str, Option<&str>)]) {
        std::fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");

        let conn = Connection::open(at).expect("open");

        conn.execute_batch(
            "CREATE TABLE InstalledBaseProducts (productId INTEGER, installationPath TEXT);
             CREATE TABLE LimitedDetails (productId INTEGER, title TEXT);",
        )
        .expect("schema");

        for (id, path, title) in rows {
            conn.execute(
                "INSERT INTO InstalledBaseProducts (productId, installationPath) VALUES (?1, ?2)",
                rusqlite::params![id, path],
            )
            .expect("insert");

            if let Some(title) = title {
                conn.execute(
                    "INSERT INTO LimitedDetails (productId, title) VALUES (?1, ?2)",
                    rusqlite::params![id, title],
                )
                .expect("insert");
            }
        }
    }

    #[test]
    fn galaxys_database_yields_installed_games() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let game = tmp.path().join("Games/Witcher 3");
        std::fs::create_dir_all(&game).expect("mkdir");

        galaxy_db(
            &tmp.path()
                .join("ProgramData/GOG.com/Galaxy/storage/galaxy-2.0.db"),
            &[(
                1207664663,
                &game.to_string_lossy(),
                Some("The Witcher 3: Wild Hunt"),
            )],
        );

        let found = scan(&DetectRoots {
            program_data: Some(tmp.path().join("ProgramData")),
            ..Default::default()
        });

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "The Witcher 3: Wild Hunt");
        assert_eq!(found[0].launcher_id.as_deref(), Some("1207664663"));
    }

    /// A Galaxy that has never been online has no cached titles. Requiring one
    /// would find nothing on exactly those machines.
    #[test]
    fn a_game_with_no_cached_title_falls_back_to_its_folder() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let game = tmp.path().join("Games/Cyberpunk 2077");
        std::fs::create_dir_all(&game).expect("mkdir");

        galaxy_db(
            &tmp.path()
                .join("ProgramData/GOG.com/Galaxy/storage/galaxy-2.0.db"),
            &[(1, &game.to_string_lossy(), None)],
        );

        let found = scan(&DetectRoots {
            program_data: Some(tmp.path().join("ProgramData")),
            ..Default::default()
        });

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Cyberpunk 2077");
    }

    #[test]
    fn a_row_pointing_at_a_folder_that_is_gone_is_skipped() {
        let tmp = tempfile::tempdir().expect("tempdir");

        galaxy_db(
            &tmp.path()
                .join("ProgramData/GOG.com/Galaxy/storage/galaxy-2.0.db"),
            &[(1, "/definitely/not/here", Some("Ghost"))],
        );

        assert!(scan(&DetectRoots {
            program_data: Some(tmp.path().join("ProgramData")),
            ..Default::default()
        })
        .is_empty());
    }

    /// Galaxy's schema is not ours and has changed across major versions. A
    /// missing table must produce no games, not an error and not a panic.
    #[test]
    fn a_database_with_an_unexpected_schema_is_ignored() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let path = tmp
            .path()
            .join("ProgramData/GOG.com/Galaxy/storage/galaxy-2.0.db");

        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");

        let conn = Connection::open(&path).expect("open");
        conn.execute_batch("CREATE TABLE Something (a INTEGER);")
            .expect("schema");

        assert!(scan(&DetectRoots {
            program_data: Some(tmp.path().join("ProgramData")),
            ..Default::default()
        })
        .is_empty());
    }

    #[test]
    fn a_file_that_is_not_a_database_is_ignored() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let path = tmp
            .path()
            .join("ProgramData/GOG.com/Galaxy/storage/galaxy-2.0.db");

        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, b"not a database").expect("write");

        assert!(scan(&DetectRoots {
            program_data: Some(tmp.path().join("ProgramData")),
            ..Default::default()
        })
        .is_empty());
    }

    #[test]
    fn offline_gog_installs_are_found_by_folder() {
        let tmp = tempfile::tempdir().expect("tempdir");

        std::fs::create_dir_all(tmp.path().join("GOG Games/Stardew Valley")).expect("mkdir");

        let found = scan(&DetectRoots {
            drives: vec![tmp.path().to_path_buf()],
            ..Default::default()
        });

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Stardew Valley");
        assert_eq!(found[0].launcher_id, None);
    }
}
