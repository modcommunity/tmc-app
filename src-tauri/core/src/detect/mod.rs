//! **Finding where games are installed**, so nobody has to type a path.
//!
//! Every mod manager's first screen is "where is your game?", and every one of
//! them is worse for it. The answer is almost always already written down —
//! Steam keeps a manifest per game, Epic keeps a JSON file per game, GOG Galaxy
//! keeps a SQLite database — so this reads what the launchers already know
//! rather than asking.
//!
//! | Source | Where it looks |
//! | --- | --- |
//! | [`steam`] | `libraryfolders.vdf`, then every `appmanifest_*.acf` |
//! | [`epic`] | `Data/Manifests/*.item`, and Heroic/Legendary on Linux |
//! | [`gog`] | `galaxy-2.0.db`, and the `GOG Games` folder |
//! | [`folders`] | Xbox, Ubisoft, EA and Battle.net's fixed layouts |
//! | hints | folders a game's own `sandbox.json` names |
//!
//! WHAT THIS IS NOT ALLOWED TO DO
//! -----------------------------
//! **Detection suggests; it never configures.** Everything here is read-only
//! and returns candidates. Applying one goes through
//! [`crate::anchor::validate_root`] exactly as a hand-typed path does, so a
//! detected folder gets no more trust than a chosen one — which matters,
//! because a game directory is a jail anchor and one pointed at the wrong place
//! is a plugin writing where it should not.
//!
//! **It reads no environment variable.** The platform directories come in
//! through [`DetectRoots`], filled by the Tauri crate from its path resolver,
//! for the same reason every other path in this app does: the environment of
//! the process that launched the app is not a trust boundary.
//!
//! MATCHING A FOLDER TO A GAME
//! ---------------------------
//! A folder is only useful once it is known to be *this* game. Three ways, in
//! descending order of confidence, all declared by the game itself in
//! `plugins/app/<slug>/sandbox.json`:
//!
//!   1. **The launcher's own id** — a Steam app id, an Epic `AppName`, a GOG
//!      product id. Exact, language-independent, and survives a rename.
//!   2. **A marker file** — `GTA5.exe`. Confirms a folder really is the game.
//!   3. **The display name**, normalised. The fallback, and the only one that
//!      can be wrong, which is why a game declaring markers must also pass (2).

pub mod epic;
pub mod folders;
pub mod gog;
pub mod steam;
pub mod vdf;
pub mod walk;

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::plugins::apps::{AppPlugins, DetectSpec};

/// The platform directories a scan needs, resolved by the caller.
///
/// Supplied rather than discovered for the same reason [`crate::plugins::
/// JailRoots`] is: working them out needs the platform's rules, which is the
/// Tauri crate's job, and passing them in is what lets every scanner here be
/// tested against a `tempfile::TempDir`.
#[derive(Debug, Clone, Default)]
pub struct DetectRoots {
    /// The user's home directory.
    pub home: Option<PathBuf>,
    /// `%APPDATA%` on Windows, `~/.config` elsewhere.
    pub config: Option<PathBuf>,
    /// `%LOCALAPPDATA%`.
    pub local_data: Option<PathBuf>,
    /// `%ProgramData%`.
    pub program_data: Option<PathBuf>,
    /// `Program Files` and `Program Files (x86)`, on every drive that has them.
    pub program_files: Vec<PathBuf>,
    /// Every drive root — `C:\`, `D:\`. Empty off Windows.
    pub drives: Vec<PathBuf>,
}

/// One game found on this machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedGame {
    /// `steam`, `epic`, `gog`, `xbox`, `ubisoft`, `ea`, `battlenet`, `folder`.
    pub source: String,
    pub name: String,
    /// Absolute, and known to exist at the time of the scan.
    pub path: String,
    /// The launcher's own identifier, when it has one.
    pub launcher_id: Option<String>,
    /// What would start it, when the launcher offers a URI for that.
    pub launch_uri: Option<String>,
    pub size_bytes: Option<u64>,
    /// The TMC app this was matched to, when a hint matched.
    pub slug: Option<String>,
}

/// The result of one scan.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectReport {
    /// Everything found, matched games first.
    pub games: Vec<DetectedGame>,
    /// Which sources produced anything, for the "we looked here" line.
    pub sources: Vec<String>,
}

/// Cap on games returned. A Steam library of 3,000 games is real, and a picker
/// showing all of them is not.
const MAX_GAMES: usize = 2_000;

/// Scan everything, and match what is found against the app's known games.
pub fn scan(roots: &DetectRoots, plugins: &AppPlugins) -> DetectReport {
    let mut games = Vec::new();

    games.extend(steam::scan(roots));
    games.extend(epic::scan(roots));
    games.extend(gog::scan(roots));
    games.extend(folders::scan(roots));
    games.extend(folders::from_hints(roots, plugins));

    dedupe(&mut games);

    let hints = hints_from(plugins);

    for game in &mut games {
        if game.slug.is_none() {
            game.slug = match_slug(game, &hints);
        }
    }

    /*
     * Matched games first, then by name. A user opening this looks for the game
     * they came to mod, and a list of 400 Steam titles with the three the app
     * actually supports scattered through it is a list nobody reads.
     */
    games.sort_by(|a, b| {
        b.slug
            .is_some()
            .cmp(&a.slug.is_some())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    games.truncate(MAX_GAMES);

    let mut sources: Vec<String> = games.iter().map(|g| g.source.clone()).collect();

    sources.sort();
    sources.dedup();

    DetectReport { games, sources }
}

/// Every game's detection hints, keyed by slug.
pub fn hints_from(plugins: &AppPlugins) -> BTreeMap<String, DetectSpec> {
    let mut out = BTreeMap::new();

    for slug in plugins.slugs() {
        if let Some(spec) = plugins.sandbox_spec(&slug) {
            if !spec.detect.is_empty() {
                out.insert(slug, spec.detect.clone());
            }
        }
    }

    out
}

/// Which TMC game is this, if any?
fn match_slug(game: &DetectedGame, hints: &BTreeMap<String, DetectSpec>) -> Option<String> {
    let normalised = normalise(&game.name);

    for (slug, spec) in hints {
        let by_id = match game.source.as_str() {
            "steam" => matches_id(&spec.steam_app_ids, game.launcher_id.as_deref()),
            "epic" => matches_id(&spec.epic_app_names, game.launcher_id.as_deref()),
            "gog" => matches_id(&spec.gog_product_ids, game.launcher_id.as_deref()),
            _ => false,
        };

        let by_name = spec
            .names
            .iter()
            .any(|candidate| normalise(candidate) == normalised);

        if !by_id && !by_name {
            continue;
        }

        /*
         * A marker settles it. Name matching is the only rule here that can be
         * wrong — two games genuinely share a name, and a folder can be named
         * anything — so a game that declares a marker file must have it. An id
         * match is exact and is not second-guessed.
         */
        if !by_id && !spec.markers.is_empty() && !has_marker(&game.path, &spec.markers) {
            continue;
        }

        return Some(slug.clone());
    }

    None
}

fn matches_id(candidates: &[String], id: Option<&str>) -> bool {
    let Some(id) = id else {
        return false;
    };

    candidates.iter().any(|c| c.eq_ignore_ascii_case(id))
}

pub(crate) fn has_marker(dir: &str, markers: &[String]) -> bool {
    let base = std::path::Path::new(dir);

    markers.iter().any(|marker| {
        // Through the jail's own join, so a marker is a relative path and
        // nothing else — a hint file is authored, but it is still a file on
        // disk that decides whether we look somewhere.
        crate::plugins::jail::join_relative(base, marker).is_ok_and(|path| path.exists())
    })
}

/// A game name reduced to something two spellings of it agree on.
///
/// `Grand Theft Auto V`, `Grand Theft Auto V - Enhanced` and `GRAND THEFT AUTO
/// V™` all reduce to the same string; `Portal` and `Portal 2` do not, because
/// digits are kept.
pub(crate) fn normalise(name: &str) -> String {
    let mut out = String::with_capacity(name.len());

    for ch in name.chars() {
        if ch.is_alphanumeric() {
            out.extend(ch.to_lowercase());
        }
    }

    // Editions and trademark noise, once the punctuation is gone.
    for suffix in [
        "gameoftheyearedition",
        "gameoftheyear",
        "definitiveedition",
        "enhancededition",
        "completeedition",
        "specialedition",
        "legendaryedition",
        "remastered",
        "goty",
        "edition",
    ] {
        if let Some(trimmed) = out.strip_suffix(suffix) {
            if !trimmed.is_empty() {
                out = trimmed.to_string();
            }
        }
    }

    out
}

/// One entry per real folder, preferring the source that knows the most.
fn dedupe(games: &mut Vec<DetectedGame>) {
    // A launcher id beats no launcher id: the same folder found through both
    // Galaxy and a folder scan should keep the row carrying the product id.
    games.sort_by_key(|g| g.launcher_id.is_none());

    let mut seen: Vec<String> = Vec::new();

    games.retain(|game| {
        let key = std::path::Path::new(&game.path)
            .canonicalize()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| game.path.clone());

        if seen.contains(&key) {
            return false;
        }

        seen.push(key);

        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_normalise_past_editions_and_punctuation() {
        assert_eq!(normalise("Grand Theft Auto V"), "grandtheftautov");
        assert_eq!(normalise("GRAND THEFT AUTO V™"), "grandtheftautov");
        assert_eq!(
            normalise("The Witcher 3: Wild Hunt - Game of the Year Edition"),
            normalise("The Witcher 3 Wild Hunt")
        );

        // A digit is part of the name, not noise.
        assert_ne!(normalise("Portal"), normalise("Portal 2"));
    }

    fn spec(steam: &[&str], names: &[&str], markers: &[&str]) -> DetectSpec {
        DetectSpec {
            steam_app_ids: steam.iter().map(|s| s.to_string()).collect(),
            names: names.iter().map(|s| s.to_string()).collect(),
            markers: markers.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    fn game(source: &str, name: &str, id: Option<&str>, path: &str) -> DetectedGame {
        DetectedGame {
            source: source.into(),
            name: name.into(),
            path: path.into(),
            launcher_id: id.map(str::to_string),
            launch_uri: None,
            size_bytes: None,
            slug: None,
        }
    }

    #[test]
    fn a_steam_app_id_matches_regardless_of_the_display_name() {
        let hints = BTreeMap::from([("gtav".to_string(), spec(&["271590"], &[], &[]))]);

        let found = game(
            "steam",
            "Grand Theft Auto V [Spanish]",
            Some("271590"),
            "/x",
        );

        assert_eq!(match_slug(&found, &hints).as_deref(), Some("gtav"));
    }

    #[test]
    fn a_name_match_needs_the_marker_when_one_is_declared() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().to_string_lossy().into_owned();

        let hints = BTreeMap::from([(
            "gtav".to_string(),
            spec(&[], &["Grand Theft Auto V"], &["GTA5.exe"]),
        )]);

        let found = game("folder", "Grand Theft Auto V", None, &path);

        assert_eq!(
            match_slug(&found, &hints),
            None,
            "a folder without the marker is not the game"
        );

        std::fs::write(tmp.path().join("GTA5.exe"), b"").expect("write");

        assert_eq!(match_slug(&found, &hints).as_deref(), Some("gtav"));
    }

    /// An id match is exact, so it is not second-guessed by a marker the user
    /// may simply not have (a game mid-update, a partial verify).
    #[test]
    fn an_id_match_does_not_need_the_marker() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let hints = BTreeMap::from([("gtav".to_string(), spec(&["271590"], &[], &["GTA5.exe"]))]);

        let found = game(
            "steam",
            "Whatever",
            Some("271590"),
            &tmp.path().to_string_lossy(),
        );

        assert_eq!(match_slug(&found, &hints).as_deref(), Some("gtav"));
    }

    #[test]
    fn a_marker_cannot_escape_the_folder_it_checks() {
        let tmp = tempfile::tempdir().expect("tempdir");

        std::fs::write(tmp.path().join("real.exe"), b"").expect("write");

        let inner = tmp.path().join("game");
        std::fs::create_dir_all(&inner).expect("mkdir");

        assert!(!has_marker(&inner.to_string_lossy(), &["real.exe".into()]));
        assert!(!has_marker(
            &inner.to_string_lossy(),
            &["../real.exe".into()]
        ));
        assert!(!has_marker(
            &inner.to_string_lossy(),
            &["/etc/passwd".into()]
        ));
    }

    #[test]
    fn the_same_folder_from_two_sources_appears_once_keeping_the_richer_row() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().to_string_lossy().into_owned();

        let mut games = vec![
            game("gog", "Witcher 3", None, &path),
            game("gog", "The Witcher 3", Some("1207664663"), &path),
        ];

        dedupe(&mut games);

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].launcher_id.as_deref(), Some("1207664663"));
    }
}
