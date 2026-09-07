//! The launchers with no metadata worth reading, and the folders a game names
//! for itself.
//!
//! Steam, Epic and GOG all keep a machine-readable record of what they have
//! installed. These do not:
//!
//!   * **Xbox / Microsoft Store** puts games under `<drive>:\XboxGames\<Name>\
//!     Content`. The real manifest lives in `WindowsApps`, which is ACL'd so
//!     that even an administrator cannot list it without taking ownership —
//!     so the folder layout is the only thing readable.
//!   * **Ubisoft Connect** installs into `Ubisoft Game Launcher\games\<Name>`
//!     by default, with the authoritative list in the registry.
//!   * **EA app / Origin** uses `EA Games\<Name>` and `Origin Games\<Name>`.
//!   * **Battle.net** installs each game straight into Program Files.
//!
//! So this is a folder scan, one level deep, over a fixed list of parents. It
//! is deliberately shallow: a recursive walk of Program Files is slow, finds
//! hundreds of things that are not games, and on Windows spends most of its
//! time in directories it is not allowed to read.
//!
//! A folder found this way carries no launcher id, so it can only ever be
//! matched to a game by name and marker file — which is why a game that cares
//! about being found here should declare a marker in its `sandbox.json`.

use std::path::{Path, PathBuf};

use crate::plugins::apps::AppPlugins;

use super::{DetectRoots, DetectedGame};

/// Cap on entries read from one parent folder.
const MAX_ENTRIES: usize = 500;

/// A parent folder, and what the games inside it came from.
struct Parent {
    source: &'static str,
    path: PathBuf,
    /// Games are one level further down, in a subfolder of this name — Xbox's
    /// `Content`, and nothing else so far.
    inner: Option<&'static str>,
}

fn parents(roots: &DetectRoots) -> Vec<Parent> {
    let mut out: Vec<Parent> = Vec::new();

    for drive in &roots.drives {
        out.push(Parent {
            source: "xbox",
            path: drive.join("XboxGames"),
            inner: Some("Content"),
        });
    }

    for base in &roots.program_files {
        for (source, rel) in [
            ("ubisoft", "Ubisoft/Ubisoft Game Launcher/games"),
            ("ea", "EA Games"),
            ("ea", "Origin Games"),
            ("battlenet", "Battle.net"),
        ] {
            out.push(Parent {
                source,
                path: base.join(rel),
                inner: None,
            });
        }
    }

    if let Some(home) = &roots.home {
        // The EA app on macOS, and Ubisoft's Wine prefix layout on Linux, both
        // end up under the user's own directory.
        out.push(Parent {
            source: "ea",
            path: home.join("Applications/EA Games"),
            inner: None,
        });
    }

    out.retain(|parent| parent.path.is_dir());

    out
}

/// Every game under a launcher folder with no readable metadata.
pub fn scan(roots: &DetectRoots) -> Vec<DetectedGame> {
    let mut out = Vec::new();

    for parent in parents(roots) {
        let Ok(entries) = std::fs::read_dir(&parent.path) else {
            continue;
        };

        for entry in entries.flatten().take(MAX_ENTRIES) {
            let mut path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            if name.is_empty() || name.starts_with('.') {
                continue;
            }

            if let Some(inner) = parent.inner {
                let deeper = path.join(inner);

                // Xbox's `<Name>\Content` is where the game actually is. A
                // folder without it is a leftover, not an install.
                if !deeper.is_dir() {
                    continue;
                }

                path = deeper;
            }

            out.push(DetectedGame {
                source: parent.source.to_string(),
                name,
                path: path.to_string_lossy().into_owned(),
                launcher_id: None,
                launch_uri: None,
                size_bytes: None,
                slug: None,
            });
        }
    }

    out
}

/// Folders a game names for itself in `plugins/app/<slug>/sandbox.json`.
///
/// The escape hatch for everything the launchers do not cover: `~/.minecraft`,
/// a dedicated server unpacked wherever somebody unpacked it, a game bought
/// outside any store. These come back with their `slug` already set, because
/// the game named the folder — there is nothing left to match.
pub fn from_hints(roots: &DetectRoots, plugins: &AppPlugins) -> Vec<DetectedGame> {
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else {
        "linux"
    };

    let mut out = Vec::new();

    for slug in plugins.slugs() {
        let Some(spec) = plugins.sandbox_spec(&slug) else {
            continue;
        };

        let mut candidates: Vec<String> =
            spec.detect.paths.get(platform).cloned().unwrap_or_default();

        // `any` applies everywhere, for the games whose folder is the same on
        // every platform relative to home.
        candidates.extend(spec.detect.paths.get("any").cloned().unwrap_or_default());

        for candidate in candidates {
            for path in expand(&candidate, roots) {
                if !path.is_dir() {
                    continue;
                }

                if !spec.detect.markers.is_empty()
                    && !super::has_marker(&path.to_string_lossy(), &spec.detect.markers)
                {
                    continue;
                }

                out.push(DetectedGame {
                    source: "folder".into(),
                    name: path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| slug.clone()),
                    path: path.to_string_lossy().into_owned(),
                    launcher_id: None,
                    launch_uri: None,
                    size_bytes: None,
                    slug: Some(slug.clone()),
                });
            }
        }
    }

    out
}

/// Turn one hint into the real paths it could mean.
///
/// `~/` is the user's home. A leading `<drives>/` is every drive root, which is
/// how a hint says "wherever this is on Windows" without naming `C:`. Anything
/// else is taken literally, and a hint that is not absolute after expansion is
/// dropped rather than resolved against the working directory — which is not a
/// meaningful location for a GUI app.
fn expand(hint: &str, roots: &DetectRoots) -> Vec<PathBuf> {
    if hint.len() > 512 || hint.contains('\0') {
        return Vec::new();
    }

    if let Some(rest) = hint.strip_prefix("~/") {
        return roots
            .home
            .iter()
            .filter_map(|home| join_checked(home, rest))
            .collect();
    }

    if let Some(rest) = hint.strip_prefix("<drives>/") {
        return roots
            .drives
            .iter()
            .filter_map(|drive| join_checked(drive, rest))
            .collect();
    }

    if let Some(rest) = hint.strip_prefix("<programFiles>/") {
        return roots
            .program_files
            .iter()
            .filter_map(|base| join_checked(base, rest))
            .collect();
    }

    if let Some(rest) = hint.strip_prefix("<appData>/") {
        return roots
            .config
            .iter()
            .filter_map(|base| join_checked(base, rest))
            .collect();
    }

    if let Some(rest) = hint.strip_prefix("<localAppData>/") {
        return roots
            .local_data
            .iter()
            .filter_map(|base| join_checked(base, rest))
            .collect();
    }

    let path = PathBuf::from(hint);

    if path.is_absolute() {
        vec![path]
    } else {
        Vec::new()
    }
}

/// Join a hint's remainder onto a base, refusing anything that climbs out.
///
/// The same `join_relative` the plugin jail uses. A hint is authored rather
/// than hostile, but `~/../../etc` producing a candidate the user might then
/// accept as a game folder is not a mistake worth being able to make.
fn join_checked(base: &Path, rest: &str) -> Option<PathBuf> {
    crate::plugins::jail::join_relative(base, rest).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xbox_games_are_found_one_level_down_in_content() {
        let tmp = tempfile::tempdir().expect("tempdir");

        std::fs::create_dir_all(tmp.path().join("XboxGames/Forza Horizon 5/Content"))
            .expect("mkdir");
        // A leftover with no Content folder is not an install.
        std::fs::create_dir_all(tmp.path().join("XboxGames/Leftover")).expect("mkdir");

        let found = scan(&DetectRoots {
            drives: vec![tmp.path().to_path_buf()],
            ..Default::default()
        });

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Forza Horizon 5");
        assert_eq!(found[0].source, "xbox");
        assert!(found[0].path.ends_with("Content"));
    }

    #[test]
    fn ubisoft_and_ea_folders_are_scanned() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let pf = tmp.path().join("Program Files");

        std::fs::create_dir_all(pf.join("Ubisoft/Ubisoft Game Launcher/games/Anno 1800"))
            .expect("mkdir");
        std::fs::create_dir_all(pf.join("EA Games/Dragon Age")).expect("mkdir");

        let found = scan(&DetectRoots {
            program_files: vec![pf],
            ..Default::default()
        });

        let names: Vec<&str> = found.iter().map(|g| g.name.as_str()).collect();

        assert!(names.contains(&"Anno 1800"), "{names:?}");
        assert!(names.contains(&"Dragon Age"), "{names:?}");
    }

    #[test]
    fn a_home_relative_hint_expands() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let expanded = expand(
            "~/.minecraft",
            &DetectRoots {
                home: Some(tmp.path().to_path_buf()),
                ..Default::default()
            },
        );

        assert_eq!(expanded, vec![tmp.path().join(".minecraft")]);
    }

    #[test]
    fn a_drive_relative_hint_expands_to_every_drive() {
        let expanded = expand(
            "<drives>/Games/Server",
            &DetectRoots {
                drives: vec![PathBuf::from("/c"), PathBuf::from("/d")],
                ..Default::default()
            },
        );

        assert_eq!(
            expanded,
            vec![
                PathBuf::from("/c/Games/Server"),
                PathBuf::from("/d/Games/Server")
            ]
        );
    }

    #[test]
    fn a_hint_cannot_climb_out_of_the_base_it_expands_against() {
        let roots = DetectRoots {
            home: Some(PathBuf::from("/home/player")),
            ..Default::default()
        };

        for bad in ["~/../../etc", "~/a/../../../root", "relative/path"] {
            assert!(expand(bad, &roots).is_empty(), "{bad} should not expand");
        }
    }

    #[test]
    fn an_absolute_hint_is_taken_literally() {
        let expanded = expand("/opt/minecraft-server", &DetectRoots::default());

        assert_eq!(expanded, vec![PathBuf::from("/opt/minecraft-server")]);
    }
}
