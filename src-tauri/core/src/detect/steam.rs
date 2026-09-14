//! Finding games Steam has installed.
//!
//! Steam keeps two things this needs, both in plain KeyValues text:
//!
//!   * **`steamapps/libraryfolders.vdf`** — every library folder, including the
//!     ones on other drives. There is no other way to learn about a second
//!     library, and assuming there is only one is why so many tools "cannot
//!     find" a game that is plainly installed.
//!   * **`steamapps/appmanifest_<id>.acf`**, one per installed game — its app
//!     id, its display name and its `installdir`, which is a folder name under
//!     `steamapps/common/` and is frequently NOT the same as the display name
//!     ("Grand Theft Auto V" vs "GTAV" is the usual example).
//!
//! Reading the manifests rather than listing `steamapps/common/` matters for
//! two reasons: a folder left behind by an uninstall is not an installed game,
//! and the manifest is the only place the numeric app id — the thing that
//! actually identifies a game across languages and renames — appears.

use std::path::{Path, PathBuf};

use super::vdf::{self, Value};
use super::{DetectRoots, DetectedGame};

/// Cap on library folders followed. Real installs have one to five.
const MAX_LIBRARIES: usize = 32;

/// Cap on manifests read per library.
const MAX_MANIFESTS: usize = 4_000;

/// Every Steam root worth looking in on this machine.
///
/// Several, because Linux has a genuine mess of them: the classic
/// `~/.steam/steam` symlink, the XDG `~/.local/share/Steam` it usually points
/// at, the Flatpak sandbox, and the Snap. A user with Flatpak Steam and a
/// leftover native `~/.steam` has both, and only one holds their games.
pub fn roots(roots: &DetectRoots) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();

    if let Some(home) = &roots.home {
        for rel in [
            // Linux
            ".steam/steam",
            ".steam/root",
            ".local/share/Steam",
            ".var/app/com.valvesoftware.Steam/data/Steam",
            "snap/steam/common/.local/share/Steam",
            // macOS
            "Library/Application Support/Steam",
        ] {
            out.push(home.join(rel));
        }
    }

    for base in &roots.program_files {
        out.push(base.join("Steam"));
    }

    // A Steam installed outside Program Files, which is common on a drive kept
    // for games.
    for drive in &roots.drives {
        out.push(drive.join("Steam"));
    }

    out.retain(|path| path.join("steamapps").is_dir());

    dedupe(out)
}

/// Every library folder Steam knows about, including the root itself.
fn libraries(steam_root: &Path) -> Vec<PathBuf> {
    let mut out = vec![steam_root.to_path_buf()];

    let vdf_path = steam_root.join("steamapps/libraryfolders.vdf");

    let Some(parsed) = vdf::parse_file(&vdf_path) else {
        return out;
    };

    let Some(folders) = parsed.get("libraryfolders").and_then(Value::as_map) else {
        return out;
    };

    for entry in folders.values().take(MAX_LIBRARIES) {
        /*
         * Two shapes have shipped. Modern clients write a block per library
         * with a `path` inside it; clients before 2021 wrote the path as the
         * value directly. Handling both costs one line and is the difference
         * between finding a second library and not.
         */
        let path = match entry {
            Value::Str(path) => Some(path.as_str()),
            Value::Map(_) => entry.str_at("path"),
        };

        if let Some(path) = path {
            out.push(PathBuf::from(path));
        }
    }

    dedupe(out)
}

/// Scan every Steam library on this machine.
pub fn scan(roots: &DetectRoots) -> Vec<DetectedGame> {
    let mut out = Vec::new();

    for steam_root in self::roots(roots) {
        for library in libraries(&steam_root) {
            scan_library(&library, &mut out);
        }
    }

    out
}

fn scan_library(library: &Path, out: &mut Vec<DetectedGame>) {
    let steamapps = library.join("steamapps");

    let Ok(entries) = std::fs::read_dir(&steamapps) else {
        return;
    };

    let common = steamapps.join("common");

    for entry in entries.flatten().take(MAX_MANIFESTS) {
        let path = entry.path();

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
            continue;
        }

        let Some(parsed) = vdf::parse_file(&path) else {
            continue;
        };

        let Some(state) = parsed.get("AppState") else {
            continue;
        };

        let Some(install_dir) = state.str_at("installdir") else {
            continue;
        };

        let Some(app_id) = state.str_at("appid") else {
            continue;
        };

        let dir = common.join(install_dir);

        /*
         * A manifest whose folder is gone is not an installed game. Steam
         * leaves these behind after a failed uninstall and after moving a game
         * between libraries, and offering one produces a game directory that
         * fails every check the moment somebody accepts it.
         */
        if !dir.is_dir() {
            continue;
        }

        out.push(DetectedGame {
            source: "steam".into(),
            name: state
                .str_at("name")
                .unwrap_or(install_dir)
                .trim()
                .to_string(),
            path: dir.to_string_lossy().into_owned(),
            launcher_id: Some(app_id.to_string()),
            // The URI that starts a Steam game without Steam having to be told
            // where it is. `rungameid` rather than `run` because it also works
            // for shortcuts and for games with several launch options.
            launch_uri: Some(format!("steam://rungameid/{app_id}")),
            size_bytes: state
                .str_at("SizeOnDisk")
                .and_then(|s| s.parse::<u64>().ok()),
            slug: None,
        });
    }
}

/// Keep the first of each path, preserving order.
///
/// **The canonical form is the KEY, not the answer.** Pushing the resolved
/// path instead of the one Steam wrote down is the difference between a
/// library at `F:\SteamLibrary` and one at `\\?\F:\SteamLibrary`, because
/// `canonicalize` on Windows returns the verbatim spelling — and every game
/// path is built by joining onto this, so the prefix ends up on all of them,
/// in the scan list, in `settings.json` and in front of the user. What is
/// wanted from canonicalising here is only the ANSWER to "are these two the
/// same directory?"; the path itself is already fine as Steam spelled it.
fn dedupe(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::with_capacity(paths.len());
    let mut out: Vec<PathBuf> = Vec::with_capacity(paths.len());

    for path in paths {
        // Canonicalise so `~/.steam/steam` and `~/.local/share/Steam` — which
        // are the same directory through a symlink on most Linux installs — do
        // not both get scanned.
        let key = crate::canon::canonicalize_or_keep(&path);

        if !seen.contains(&key) {
            seen.push(key);
            out.push(path);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    /// A whole fake Steam install: two libraries, three manifests, one of which
    /// names a folder that is not there.
    fn fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        let steam = root.join("Steam");
        let other = root.join("SteamLibrary");

        write(
            &steam.join("steamapps/libraryfolders.vdf"),
            &format!(
                r#"
"libraryfolders"
{{
    "0" {{ "path" "{}" }}
    "1" {{ "path" "{}" }}
}}
"#,
                steam.display(),
                other.display()
            ),
        );

        write(
            &steam.join("steamapps/appmanifest_271590.acf"),
            r#""AppState" { "appid" "271590" "name" "Grand Theft Auto V" "installdir" "Grand Theft Auto V" "SizeOnDisk" "94489280512" }"#,
        );

        write(
            &steam.join("steamapps/appmanifest_9999.acf"),
            r#""AppState" { "appid" "9999" "name" "Uninstalled Game" "installdir" "Gone" }"#,
        );

        write(
            &other.join("steamapps/appmanifest_252490.acf"),
            r#""AppState" { "appid" "252490" "name" "Rust" "installdir" "Rust" }"#,
        );

        std::fs::create_dir_all(steam.join("steamapps/common/Grand Theft Auto V")).expect("mkdir");
        std::fs::create_dir_all(other.join("steamapps/common/Rust")).expect("mkdir");

        tmp
    }

    fn roots_for(tmp: &tempfile::TempDir) -> DetectRoots {
        DetectRoots {
            drives: vec![tmp.path().to_path_buf()],
            ..Default::default()
        }
    }

    #[test]
    fn games_in_a_second_library_are_found() {
        let tmp = fixture();

        let found = scan(&roots_for(&tmp));

        let names: Vec<&str> = found.iter().map(|g| g.name.as_str()).collect();

        assert!(names.contains(&"Grand Theft Auto V"), "{names:?}");
        assert!(
            names.contains(&"Rust"),
            "a game on a second library must be found: {names:?}"
        );
    }

    /// A library is reported in the spelling Steam wrote down.
    ///
    /// `dedupe` canonicalises to answer "are these the same directory?", and
    /// used to return that resolved form. On Windows that is `\\?\F:\…`, and
    /// since every game path is joined onto a library path the prefix reached
    /// the scan list, `settings.json`, the Library row and every audit line.
    #[test]
    fn a_library_keeps_the_spelling_it_was_written_with() {
        let tmp = fixture();
        let steam = tmp.path().join("Steam");

        let found = scan(&roots_for(&tmp));

        let gta = found
            .iter()
            .find(|g| g.name == "Grand Theft Auto V")
            .expect("found");

        assert_eq!(
            gta.path,
            steam
                .join("steamapps/common/Grand Theft Auto V")
                .to_string_lossy()
                .into_owned()
        );
    }

    /// The half of `dedupe` that canonicalising is actually for: two spellings
    /// of one directory are one library, and the first one wins.
    #[cfg(unix)]
    #[test]
    fn two_paths_to_one_directory_are_scanned_once() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let real = tmp.path().join("real");
        let link = tmp.path().join("link");

        std::fs::create_dir_all(&real).expect("mkdir");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let deduped = dedupe(vec![link.clone(), real.clone()]);

        assert_eq!(deduped, vec![link]);
    }

    /// Steam leaves manifests behind after a failed uninstall. Offering one
    /// produces a game folder that fails every check the moment it is accepted.
    #[test]
    fn a_manifest_whose_folder_is_gone_is_not_offered() {
        let tmp = fixture();

        let found = scan(&roots_for(&tmp));

        assert!(!found.iter().any(|g| g.name == "Uninstalled Game"));
    }

    /// `installdir` is not the display name, and using the display name as a
    /// folder is the single most common way to look in the wrong place.
    #[test]
    fn the_install_folder_comes_from_installdir_not_the_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let steam = tmp.path().join("Steam");

        write(
            &steam.join("steamapps/appmanifest_1.acf"),
            r#""AppState" { "appid" "1" "name" "Grand Theft Auto V" "installdir" "GTAV" }"#,
        );

        std::fs::create_dir_all(steam.join("steamapps/common/GTAV")).expect("mkdir");

        let found = scan(&DetectRoots {
            drives: vec![tmp.path().to_path_buf()],
            ..Default::default()
        });

        assert_eq!(found.len(), 1);
        assert!(found[0].path.ends_with("GTAV"));
        assert_eq!(found[0].name, "Grand Theft Auto V");
        assert_eq!(found[0].launcher_id.as_deref(), Some("1"));
        assert_eq!(found[0].launch_uri.as_deref(), Some("steam://rungameid/1"));
    }

    #[test]
    fn a_machine_with_no_steam_finds_nothing_and_does_not_fail() {
        let tmp = tempfile::tempdir().expect("tempdir");

        assert!(scan(&DetectRoots {
            home: Some(tmp.path().to_path_buf()),
            drives: vec![tmp.path().to_path_buf()],
            ..Default::default()
        })
        .is_empty());
    }
}
