//! Finding games the Epic Games Launcher has installed.
//!
//! Epic writes one JSON manifest per installed game into a shared directory,
//! and each carries the install path outright:
//!
//! ```text
//! C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests\<hash>.item
//! /Users/Shared/Epic/EpicGamesLauncher/Data/Manifests/<hash>.item
//! ```
//!
//! Simpler than Steam's — there is no library list to follow, because the
//! manifest names an absolute `InstallLocation` wherever it happens to be.
//!
//! There is no official Epic launcher on Linux. Heroic and Legendary both
//! reimplement it and both keep their own metadata; [`heroic_roots`] covers
//! Heroic, which is what a Linux user with Epic games almost certainly has.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{DetectRoots, DetectedGame};

/// Cap on manifests read. A large Epic library is a few hundred.
const MAX_MANIFESTS: usize = 2_000;

/// Cap on one manifest. They are a few kilobytes.
const MAX_BYTES: u64 = 1024 * 1024;

/// The fields worth reading. Epic's manifests carry around forty; the rest
/// describe chunked-download state that means nothing once a game is installed.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Manifest {
    #[serde(default)]
    install_location: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    /// The launcher's own identifier — what `com.epicgames.launcher://apps/…`
    /// takes.
    #[serde(default)]
    app_name: Option<String>,
    #[serde(default)]
    install_size: Option<u64>,
    /// Present and true while a game is still downloading.
    ///
    /// Named explicitly rather than left to `rename_all`: Epic's Hungarian
    /// prefix makes this the one field in the file that is not PascalCase, so
    /// the derived name would be `BIsIncompleteInstall` and would never match —
    /// which reads as "no game is ever incomplete" and offers a folder that is
    /// still being written into.
    #[serde(default, rename = "bIsIncompleteInstall")]
    b_is_incomplete_install: Option<bool>,
}

/// Where the launcher keeps its manifests on this machine.
pub fn manifest_dirs(roots: &DetectRoots) -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Some(program_data) = &roots.program_data {
        out.push(program_data.join("Epic/EpicGamesLauncher/Data/Manifests"));
    }

    // macOS keeps it in the shared user's folder rather than in ProgramData.
    out.push(PathBuf::from(
        "/Users/Shared/Epic/EpicGamesLauncher/Data/Manifests",
    ));

    for drive in &roots.drives {
        out.push(drive.join("ProgramData/Epic/EpicGamesLauncher/Data/Manifests"));
    }

    out.retain(|dir| dir.is_dir());

    out
}

/// Heroic's own install list, for Linux and for anybody using it elsewhere.
fn heroic_roots(roots: &DetectRoots) -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Some(config) = &roots.config {
        out.push(config.join("heroic/legendaryConfig/legendary/installed.json"));
        out.push(config.join("heroic/store_cache/legendary_library.json"));
    }

    if let Some(home) = &roots.home {
        out.push(home.join(".config/heroic/legendaryConfig/legendary/installed.json"));
        out.push(home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic/legendaryConfig/legendary/installed.json"));
        out.push(home.join(".config/legendary/installed.json"));
    }

    out.retain(|path| path.is_file());

    out
}

pub fn scan(roots: &DetectRoots) -> Vec<DetectedGame> {
    let mut out = Vec::new();

    for dir in manifest_dirs(roots) {
        scan_manifests(&dir, &mut out);
    }

    for path in heroic_roots(roots) {
        scan_legendary(&path, &mut out);
    }

    out
}

fn scan_manifests(dir: &Path, out: &mut Vec<DetectedGame>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten().take(MAX_MANIFESTS) {
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("item") {
            continue;
        }

        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
            continue;
        }

        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };

        let Ok(manifest) = serde_json::from_str::<Manifest>(&raw) else {
            continue;
        };

        // A download still in progress is not an installed game, and pointing
        // a sandbox at one produces a folder that changes under it.
        if manifest.b_is_incomplete_install == Some(true) {
            continue;
        }

        let Some(location) = manifest.install_location else {
            continue;
        };

        let dir = PathBuf::from(&location);

        if !dir.is_dir() {
            continue;
        }

        let name = manifest
            .display_name
            .unwrap_or_else(|| {
                dir.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| location.clone())
            })
            .trim()
            .to_string();

        out.push(DetectedGame {
            source: "epic".into(),
            name,
            path: location,
            launch_uri: manifest.app_name.as_ref().map(|app| {
                format!("com.epicgames.launcher://apps/{app}?action=launch&silent=true")
            }),
            launcher_id: manifest.app_name,
            size_bytes: manifest.install_size,
            slug: None,
        });
    }
}

/// Legendary (and therefore Heroic) keeps one JSON object of installed games,
/// keyed by app name.
fn scan_legendary(path: &Path, out: &mut Vec<DetectedGame>) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES * 8) {
        return;
    }

    let Ok(raw) = std::fs::read_to_string(path) else {
        return;
    };

    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };

    let Some(entries) = parsed.as_object() else {
        return;
    };

    for (app_name, value) in entries.iter().take(MAX_MANIFESTS) {
        let Some(install_path) = value
            .get("install_path")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };

        if !Path::new(install_path).is_dir() {
            continue;
        }

        out.push(DetectedGame {
            source: "epic".into(),
            name: value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(app_name)
                .trim()
                .to_string(),
            path: install_path.to_string(),
            launcher_id: Some(app_name.clone()),
            // Heroic's own scheme, not Epic's — the Epic launcher is not what
            // would start it on this machine.
            launch_uri: Some(format!("heroic://launch/legendary/{app_name}")),
            size_bytes: value
                .get("install_size")
                .and_then(serde_json::Value::as_u64),
            slug: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_epic_manifest_yields_its_install_location() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let manifests = tmp
            .path()
            .join("ProgramData/Epic/EpicGamesLauncher/Data/Manifests");

        let game = tmp.path().join("Games/Fortnite");

        std::fs::create_dir_all(&manifests).expect("mkdir");
        std::fs::create_dir_all(&game).expect("mkdir");

        std::fs::write(
            manifests.join("abc.item"),
            serde_json::json!({
                "DisplayName": "Fortnite",
                "AppName": "Fortnite",
                "InstallLocation": game.to_string_lossy(),
                "InstallSize": 90_000_000_000u64,
            })
            .to_string(),
        )
        .expect("write");

        let found = scan(&DetectRoots {
            program_data: Some(tmp.path().join("ProgramData")),
            ..Default::default()
        });

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Fortnite");
        assert_eq!(found[0].source, "epic");
        assert!(found[0]
            .launch_uri
            .as_deref()
            .is_some_and(|u| u.starts_with("com.epicgames.launcher://")));
    }

    #[test]
    fn a_download_still_in_progress_is_not_offered() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let manifests = tmp
            .path()
            .join("ProgramData/Epic/EpicGamesLauncher/Data/Manifests");

        let game = tmp.path().join("Games/Partial");

        std::fs::create_dir_all(&manifests).expect("mkdir");
        std::fs::create_dir_all(&game).expect("mkdir");

        std::fs::write(
            manifests.join("abc.item"),
            serde_json::json!({
                "DisplayName": "Partial",
                "InstallLocation": game.to_string_lossy(),
                "bIsIncompleteInstall": true,
            })
            .to_string(),
        )
        .expect("write");

        assert!(scan(&DetectRoots {
            program_data: Some(tmp.path().join("ProgramData")),
            ..Default::default()
        })
        .is_empty());
    }

    #[test]
    fn heroics_install_list_is_read_on_linux() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let config = tmp.path().join(".config");
        let installed = config.join("heroic/legendaryConfig/legendary/installed.json");
        let game = tmp.path().join("Games/Alan Wake");

        std::fs::create_dir_all(installed.parent().expect("parent")).expect("mkdir");
        std::fs::create_dir_all(&game).expect("mkdir");

        std::fs::write(
            &installed,
            serde_json::json!({
                "AlanWake": {
                    "title": "Alan Wake",
                    "install_path": game.to_string_lossy(),
                    "install_size": 1234u64,
                }
            })
            .to_string(),
        )
        .expect("write");

        let found = scan(&DetectRoots {
            config: Some(config),
            ..Default::default()
        });

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Alan Wake");
        assert_eq!(found[0].launcher_id.as_deref(), Some("AlanWake"));
    }

    #[test]
    fn broken_json_is_skipped_rather_than_failing_the_scan() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let manifests = tmp
            .path()
            .join("ProgramData/Epic/EpicGamesLauncher/Data/Manifests");

        std::fs::create_dir_all(&manifests).expect("mkdir");
        std::fs::write(manifests.join("bad.item"), "{ not json").expect("write");

        let found = scan(&DetectRoots {
            program_data: Some(tmp.path().join("ProgramData")),
            ..Default::default()
        });

        assert!(found.is_empty());
    }
}
