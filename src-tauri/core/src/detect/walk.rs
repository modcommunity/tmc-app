//! **Looking for games where no launcher wrote anything down.**
//!
//! [`super::scan`] reads what Steam, Epic and GOG already know, which is the
//! right first answer and covers most machines. It cannot cover a game copied
//! from another PC, a dedicated server unpacked by hand, a pirate-free but
//! launcher-free GOG offline installer, or a drive that was moved. For those,
//! somebody has to look — and the only thing that knows where to look is the
//! person sitting there.
//!
//! So this walks folders **the user explicitly ticked** and nothing else.
//!
//! WHY THE ROOTS COME FROM THE USER AND ARE NOT DISCOVERED
//! -----------------------------------------------------
//! A scan of "the filesystem" is a scan of somebody's home directory, their
//! documents, their photos and every network share the machine has mounted. The
//! app has no business enumerating any of that, and a feature that did it by
//! default would be indistinguishable from one that was looking for something
//! else. `roots` is therefore a required argument with no default, and
//! [`WalkLimits::MAX_ROOTS`] bounds how many can be handed over at once.
//!
//! It is still a widening of the rule in `commands/mod.rs`, and it is the same
//! widening `commands::fs` already took for the folder picker — the webview can
//! name a directory and learn about its subdirectories. What it adds is DEPTH,
//! which is why every axis below is bounded rather than merely large.
//!
//! WHAT BOUNDS IT
//! -------------
//! Every one of these is a real machine, not a hypothetical:
//!
//!   * **Depth** ([`WalkLimits::depth`]). Games live near the top of a library
//!     folder. A depth of 4 finds `D:\Games\Steam\steamapps\common\<game>` and
//!     does not descend into a game's own asset tree, which is where the
//!     hundred thousand files are.
//!   * **Total directories** ([`WalkLimits::max_dirs`]). The stop that actually
//!     fires on a `node_modules` tree or a `/nix/store`.
//!   * **Wall clock** ([`WalkLimits::budget`]). A network share that has gone
//!     away answers `readdir` in thirty seconds per call, and no count-based
//!     limit saves a user from that.
//!   * **Symbolic links are never followed.** A link back up its own tree is
//!     how a bounded walk becomes an unbounded one, and `~/.wine/dosdevices/z:`
//!     pointing at `/` is a real layout, not a contrived one.
//!   * **Hidden directories are skipped** unless the user asked otherwise, for
//!     the reason the folder picker skips them: a dotfile tree is where the
//!     things that are none of our business live.
//!
//! WHAT IT PRODUCES
//! ---------------
//! [`DetectedGame`] rows with `source: "scan"`, matched to a slug by the same
//! rules [`super::scan`] uses — a marker file, or a folder name that normalises
//! to a declared one. **A folder with no match is not returned.** That is the
//! difference between this and a file browser: an unmatched folder is not a
//! finding, and a list of every directory on a drive is not a scan result.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::plugins::apps::{AppPlugins, DetectSpec};

use super::{has_marker, hints_from, normalise, DetectedGame};

/// How far and how long a scan may go.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkLimits {
    /// Directory levels below each root. 0 checks the root itself only.
    pub depth: usize,
    /// Directories visited across the whole scan, all roots together.
    pub max_dirs: usize,
    /// Wall-clock ceiling, in seconds.
    pub budget_secs: u64,
    /// Descend into directories whose name starts with a dot.
    ///
    /// Off by default and worth offering, because `~/.minecraft`,
    /// `~/.steam` and `~/.local/share/Steam` are all real install locations —
    /// a scan of a Linux home directory with this off finds nothing at all.
    pub hidden: bool,
}

impl WalkLimits {
    /// The most roots one scan may be given.
    ///
    /// Ten is more than any real machine has drives, and it is what stops a
    /// caller handing over a generated list of every directory in `/` and
    /// getting an unbounded scan a level at a time.
    pub const MAX_ROOTS: usize = 10;

    /// Ceilings the caller's own numbers are clamped into.
    pub const MAX_DEPTH: usize = 8;
    pub const MAX_DIRS: usize = 200_000;
    pub const MAX_BUDGET_SECS: u64 = 300;

    /// What the scan button uses when the user changes nothing.
    ///
    /// Depth 5 rather than 4: `D:\SteamLibrary\steamapps\common\<game>` is four
    /// levels below the drive root, and somebody who ticked the drive rather
    /// than the library folder is the case this exists for.
    pub fn balanced() -> Self {
        Self {
            depth: 5,
            max_dirs: 60_000,
            budget_secs: 90,
            hidden: false,
        }
    }

    fn clamped(&self) -> Self {
        Self {
            depth: self.depth.min(Self::MAX_DEPTH),
            max_dirs: self.max_dirs.clamp(1, Self::MAX_DIRS),
            budget_secs: self.budget_secs.clamp(1, Self::MAX_BUDGET_SECS),
            hidden: self.hidden,
        }
    }
}

impl Default for WalkLimits {
    fn default() -> Self {
        Self::balanced()
    }
}

/// Why a scan stopped, which the UI says out loud.
///
/// A scan that hit a limit and a scan that finished are different answers: the
/// first means "there may be more, look in a narrower folder", and presenting
/// them identically is how somebody concludes their game is not detectable when
/// the walk simply never reached it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WalkStop {
    /// Every root was walked to the depth limit.
    Completed,
    /// [`WalkLimits::max_dirs`] was reached.
    DirLimit,
    /// [`WalkLimits::budget_secs`] ran out.
    TimeLimit,
    /// The caller asked it to stop.
    Cancelled,
}

/// What one scan found.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkReport {
    pub games: Vec<DetectedGame>,
    pub dirs_visited: usize,
    pub elapsed_ms: u64,
    pub stop: WalkStop,
    /// Roots that could not be read at all, with the reason. Reported rather
    /// than dropped: "I ticked D: and got nothing" has two very different
    /// causes and the user can only act on one of them.
    pub unreadable: Vec<(String, String)>,
}

/// Called with each directory as it is entered.
///
/// Returning `false` stops the scan. This is how the UI's Cancel button works
/// and how progress reaches the screen — a scan of a whole drive takes long
/// enough that a spinner with no path under it reads as a hang.
pub type Progress<'a> = &'a mut dyn FnMut(&Path, usize) -> bool;

/// Walk the given roots, looking for anything that matches a known game.
///
/// The roots are taken as-is. Validating them as JAIL ANCHORS is deliberately
/// not done here — that check belongs to [`crate::anchor::validate_root`] and
/// applies when a folder is APPLIED, not when it is looked at. A scan is
/// read-only and reads nothing but directory names.
pub fn walk(
    roots: &[PathBuf],
    plugins: &AppPlugins,
    limits: &WalkLimits,
    progress: Progress<'_>,
) -> WalkReport {
    let limits = limits.clamped();
    let hints = hints_from(plugins);
    let started = Instant::now();
    let budget = Duration::from_secs(limits.budget_secs);

    let mut report = WalkReport {
        games: Vec::new(),
        dirs_visited: 0,
        elapsed_ms: 0,
        stop: WalkStop::Completed,
        unreadable: Vec::new(),
    };

    /*
     * Marker files are checked against every game's declared list, so the same
     * folder is not stat-ed once per game. Built once here rather than inside
     * the loop: a machine with two hundred app plugins would otherwise do two
     * hundred `exists` calls per directory, which is what turns a bounded walk
     * into a slow one.
     */
    let by_marker = marker_index(&hints);
    let deep = deep_markers(&hints);

    /*
     * A breadth-first queue rather than recursion. Depth is bounded so the
     * stack would survive, but an explicit queue is what makes "stop now"
     * answerable between any two directories — a recursive walk can only check
     * at a call boundary, and the deepest call is the one that is stuck on a
     * dead network share.
     */
    let mut queue: std::collections::VecDeque<(PathBuf, usize)> = std::collections::VecDeque::new();

    for root in roots.iter().take(WalkLimits::MAX_ROOTS) {
        match std::fs::metadata(root) {
            Ok(meta) if meta.is_dir() => queue.push_back((root.clone(), 0)),
            Ok(_) => report
                .unreadable
                .push((root.display().to_string(), "not a folder".into())),
            Err(err) => report
                .unreadable
                .push((root.display().to_string(), err.to_string())),
        }
    }

    let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();

    while let Some((dir, depth)) = queue.pop_front() {
        if report.dirs_visited >= limits.max_dirs {
            report.stop = WalkStop::DirLimit;

            break;
        }

        if started.elapsed() >= budget {
            report.stop = WalkStop::TimeLimit;

            break;
        }

        if !progress(&dir, report.dirs_visited) {
            report.stop = WalkStop::Cancelled;

            break;
        }

        report.dirs_visited += 1;

        if let Some(found) = identify(&dir, &hints, &by_marker, &deep) {
            report.games.push(found);

            /*
             * A matched folder is not descended into. Everything below a game's
             * root is the game's own data — that is where the file count lives,
             * and a second match inside it would be a subfolder that happens to
             * share a name.
             */
            continue;
        }

        if depth >= limits.depth {
            continue;
        }

        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                // Only reported for a ROOT the user chose. A permission error
                // three levels down is normal on every OS and listing them all
                // would bury the one that matters.
                if depth == 0 {
                    report
                        .unreadable
                        .push((dir.display().to_string(), err.to_string()));
                }

                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();

            /*
             * `file_type` rather than `metadata`: it does not follow the link,
             * which is the whole point. A symlink pointing back up its own tree
             * turns a depth-bounded walk into one that visits the same folders
             * at every level, and `~/.wine/dosdevices/z: -> /` is a layout
             * people really have.
             */
            let Ok(kind) = entry.file_type() else {
                continue;
            };

            if !kind.is_dir() || kind.is_symlink() {
                continue;
            }

            let name = entry.file_name();
            let name = name.to_string_lossy();

            if !limits.hidden && name.starts_with('.') {
                continue;
            }

            if is_noise(&name) {
                continue;
            }

            /*
             * A path visited once is not visited again. Two ticked roots that
             * nest — `D:\` and `D:\Games` — would otherwise walk the second
             * one twice and report every game in it twice.
             */
            if !seen.insert(path.clone()) {
                continue;
            }

            queue.push_back((path, depth + 1));
        }
    }

    report.elapsed_ms = started.elapsed().as_millis() as u64;

    dedupe_scan(&mut report.games);

    report
}

/// Folders that are never a game and are always enormous.
///
/// A deny-list rather than an allow-list, because the point of this scan is to
/// find installs in places nobody predicted. These four are named because they
/// are the ones that actually blow the directory budget on real machines before
/// the walk reaches anything interesting.
fn is_noise(name: &str) -> bool {
    const NOISE: &[&str] = &[
        "node_modules",
        "$recycle.bin",
        "system volume information",
        "windows",
        "proc",
        "sys",
        "dev",
    ];

    let lower = name.to_ascii_lowercase();

    NOISE.contains(&lower.as_str())
}

/// Marker file name → the games that declare it, for markers that ARE a name.
///
/// The fast path. A folder's own entries are read once and each name looked up
/// here, so a machine with two hundred app plugins costs one `read_dir` per
/// directory rather than two hundred `exists` calls.
///
/// Multi-segment markers are deliberately absent: `bin/x64/Game.exe` describes
/// a file BELOW the game's folder, so no entry of that folder is ever named it
/// and no name-based index can find it. Those go to [`deep_markers`].
///
/// Lower-cased because Windows and macOS are case-insensitive and a manifest
/// author writing `GTA5.exe` must match a folder holding `gta5.exe`.
fn marker_index(hints: &BTreeMap<String, DetectSpec>) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (slug, spec) in hints {
        for marker in &spec.markers {
            if marker.contains('/') || marker.contains('\\') {
                continue;
            }

            out.entry(marker.to_ascii_lowercase())
                .or_default()
                .push(slug.clone());
        }
    }

    out
}

/// The games whose markers name a path rather than a file.
///
/// Checked with [`has_marker`] per candidate directory — the same test the
/// launcher scan applies — because that is the only thing that can resolve
/// `bin/x64/Game.exe` against a folder. It is the slow path and it is bounded
/// by being rare: this is a handful of games, not the whole plugin set, and
/// without it the two ways of looking disagree about the same manifest.
fn deep_markers(hints: &BTreeMap<String, DetectSpec>) -> Vec<(String, Vec<String>)> {
    hints
        .iter()
        .filter_map(|(slug, spec)| {
            let deep: Vec<String> = spec
                .markers
                .iter()
                .filter(|m| m.contains('/') || m.contains('\\'))
                .cloned()
                .collect();

            (!deep.is_empty()).then(|| (slug.clone(), deep))
        })
        .collect()
}

/// Is this folder a game we know, and which one?
///
/// Two rules, in descending order of confidence — the same two [`super::scan`]
/// applies to a launcher's rows, minus the launcher ids, which a bare folder
/// does not have:
///
///   1. **A marker in it.** `GTA5.exe` in a folder means that folder is GTA V,
///      whatever it is called. A marker naming a PATH — `bin/x64/Game.exe` —
///      is checked with `has_marker`, the same test the launcher scan applies,
///      because no index keyed on a folder's own entry names can resolve one.
///   2. **A folder name that normalises to a declared name**, and only when the
///      game declares no marker. A game that named a marker and does not have
///      it here is not that game — that rule is what stops a folder called
///      "Rust" in somebody's code directory being offered as the game.
fn identify(
    dir: &Path,
    hints: &BTreeMap<String, DetectSpec>,
    by_marker: &BTreeMap<String, Vec<String>>,
    deep: &[(String, Vec<String>)],
) -> Option<DetectedGame> {
    let name = dir.file_name()?.to_string_lossy().into_owned();

    let mut matched: Option<String> = None;

    // (1) A marker present in this folder.
    if !by_marker.is_empty() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let file = entry.file_name();
                let file = file.to_string_lossy().to_ascii_lowercase();

                if let Some(slugs) = by_marker.get(&file) {
                    matched = slugs.first().cloned();

                    break;
                }
            }
        }
    }

    // (1b) A marker that names a path below this folder.
    if matched.is_none() {
        matched = deep
            .iter()
            .find(|(_, markers)| has_marker(&dir.to_string_lossy(), markers))
            .map(|(slug, _)| slug.clone());
    }

    // (2) The folder's own name, for games that declare no marker.
    if matched.is_none() {
        let normalised = normalise(&name);

        if !normalised.is_empty() {
            for (slug, spec) in hints {
                let by_name = spec
                    .names
                    .iter()
                    .any(|candidate| normalise(candidate) == normalised);

                if !by_name {
                    continue;
                }

                if !spec.markers.is_empty() && !has_marker(&dir.to_string_lossy(), &spec.markers) {
                    continue;
                }

                matched = Some(slug.clone());

                break;
            }
        }
    }

    let slug = matched?;

    Some(DetectedGame {
        source: "scan".into(),
        name,
        path: dir.to_string_lossy().into_owned(),
        launcher_id: None,
        launch_uri: None,
        size_bytes: None,
        slug: Some(slug),
    })
}

/// One row per game per real folder.
fn dedupe_scan(games: &mut Vec<DetectedGame>) {
    let mut seen: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();

    games.retain(|game| {
        let path = Path::new(&game.path)
            .canonicalize()
            .map(|resolved| resolved.to_string_lossy().into_owned())
            .unwrap_or_else(|_| game.path.clone());

        seen.insert((game.slug.clone().unwrap_or_default(), path))
    });

    games.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two games: one identified by a marker file, one by name only.
    const SANDBOX_JSON: &str = r#"{
        "manifestVersion": 1,
        "sandbox": {
            "deploy": {
                "defaultStrategy": "direct",
                "supportedStrategies": ["direct"]
            },
            "detect": { "markers": ["GTA5.exe"], "names": ["Grand Theft Auto V"] }
        }
    }"#;

    const NAMED_ONLY_JSON: &str = r#"{
        "manifestVersion": 1,
        "sandbox": {
            "deploy": {
                "defaultStrategy": "direct",
                "supportedStrategies": ["direct"]
            },
            "detect": { "names": ["Some Indie Game"] }
        }
    }"#;

    fn write(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);

        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    fn plugins(dir: &Path) -> AppPlugins {
        write(dir, "app/gtav/sandbox.json", SANDBOX_JSON);
        write(dir, "app/indie/sandbox.json", NAMED_ONLY_JSON);

        AppPlugins::load(dir)
    }

    /// A scan with no cancellation and no progress reporting.
    fn run(roots: &[PathBuf], plugins: &AppPlugins, limits: &WalkLimits) -> WalkReport {
        let mut noop = |_: &Path, _: usize| true;

        walk(roots, plugins, limits, &mut noop)
    }

    #[test]
    fn a_marker_identifies_a_folder_whatever_it_is_called() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        // Deliberately NOT named after the game.
        write(tmp.path(), "Games/gta-backup/GTA5.exe", "x");

        let report = run(&[tmp.path().to_path_buf()], &apps, &WalkLimits::balanced());

        assert_eq!(report.games.len(), 1, "{:?}", report.games);
        assert_eq!(report.games[0].slug.as_deref(), Some("gtav"));
        assert_eq!(report.games[0].source, "scan");
    }

    /// The rule that stops a source directory being offered as a game.
    ///
    /// A game declaring a marker and not having it here is not that game — a
    /// folder called "Grand Theft Auto V" in somebody's downloads is not an
    /// install, and offering it produces a game directory that fails every
    /// check the moment it is accepted.
    /// A marker may be a PATH, and both ways of looking must agree about it.
    ///
    /// `has_marker` resolves `bin/x64/Game.exe` correctly, so the launcher scan
    /// matches such a game; this scan compared the whole string against a
    /// directory entry's name and never did.
    #[test]
    fn a_marker_with_a_path_component_matches_and_is_re_verified() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");

        write(
            plugin_dir.path(),
            "app/deep/sandbox.json",
            r#"{
                "manifestVersion": 1,
                "sandbox": {
                    "deploy": {
                        "defaultStrategy": "direct",
                        "supportedStrategies": ["direct"]
                    },
                    "detect": { "markers": ["bin/x64/Game.exe"] }
                }
            }"#,
        );

        let apps = AppPlugins::load(plugin_dir.path());

        // The real layout the marker describes.
        write(tmp.path(), "Games/Proper/bin/x64/Game.exe", "x");

        // The same file NAME at a folder's root, which does not satisfy it.
        write(tmp.path(), "Games/Decoy/Game.exe", "x");

        let report = run(&[tmp.path().to_path_buf()], &apps, &WalkLimits::balanced());

        let paths: Vec<&str> = report.games.iter().map(|g| g.path.as_str()).collect();

        assert_eq!(report.games.len(), 1, "{paths:?}");
        assert!(paths[0].ends_with("Proper"), "{paths:?}");
    }

    #[test]
    fn a_name_match_without_the_declared_marker_is_refused() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        std::fs::create_dir_all(tmp.path().join("Grand Theft Auto V")).expect("mkdir");

        let report = run(&[tmp.path().to_path_buf()], &apps, &WalkLimits::balanced());

        assert!(report.games.is_empty(), "{:?}", report.games);
    }

    #[test]
    fn a_game_declaring_no_marker_is_matched_on_its_folder_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        std::fs::create_dir_all(tmp.path().join("Steam/common/Some Indie Game")).expect("mkdir");

        let report = run(&[tmp.path().to_path_buf()], &apps, &WalkLimits::balanced());

        assert_eq!(report.games.len(), 1, "{:?}", report.games);
        assert_eq!(report.games[0].slug.as_deref(), Some("indie"));
    }

    #[test]
    fn nothing_unmatched_is_returned() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        for name in ["Documents", "Photos", "Work/Client Files"] {
            std::fs::create_dir_all(tmp.path().join(name)).expect("mkdir");
        }

        let report = run(&[tmp.path().to_path_buf()], &apps, &WalkLimits::balanced());

        assert!(report.games.is_empty());
        // It still LOOKED, which is what distinguishes this from a scan that
        // never ran.
        assert!(report.dirs_visited > 1);
    }

    #[test]
    fn the_depth_limit_stops_the_walk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        write(tmp.path(), "a/b/c/d/e/f/deep/GTA5.exe", "x");

        let shallow = WalkLimits {
            depth: 2,
            ..WalkLimits::balanced()
        };

        assert!(run(&[tmp.path().to_path_buf()], &apps, &shallow)
            .games
            .is_empty());

        let deep = WalkLimits {
            depth: 8,
            ..WalkLimits::balanced()
        };

        assert_eq!(
            run(&[tmp.path().to_path_buf()], &apps, &deep).games.len(),
            1
        );
    }

    #[test]
    fn the_directory_budget_stops_the_walk_and_says_so() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        for i in 0..50 {
            std::fs::create_dir_all(tmp.path().join(format!("dir{i}"))).expect("mkdir");
        }

        let tight = WalkLimits {
            max_dirs: 10,
            ..WalkLimits::balanced()
        };

        let report = run(&[tmp.path().to_path_buf()], &apps, &tight);

        assert_eq!(report.stop, WalkStop::DirLimit);
        assert!(report.dirs_visited <= 10);
    }

    /// The UI's Cancel button, and the reason `walk` takes a callback.
    #[test]
    fn a_callback_can_stop_the_walk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        for i in 0..30 {
            std::fs::create_dir_all(tmp.path().join(format!("dir{i}"))).expect("mkdir");
        }

        let mut seen = 0usize;
        let mut stop_after_three = |_: &Path, _: usize| {
            seen += 1;

            seen <= 3
        };

        let report = walk(
            &[tmp.path().to_path_buf()],
            &apps,
            &WalkLimits::balanced(),
            &mut stop_after_three,
        );

        assert_eq!(report.stop, WalkStop::Cancelled);
    }

    /// A link back up its own tree is how a bounded walk becomes unbounded.
    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_not_followed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        std::fs::create_dir_all(tmp.path().join("real/inner")).expect("mkdir");
        std::os::unix::fs::symlink(tmp.path(), tmp.path().join("real/loop")).expect("symlink");

        let report = run(&[tmp.path().to_path_buf()], &apps, &WalkLimits::balanced());

        assert_eq!(report.stop, WalkStop::Completed, "the loop was followed");
    }

    #[test]
    fn hidden_directories_are_skipped_unless_asked_for() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        write(tmp.path(), ".local/share/gta/GTA5.exe", "x");

        assert!(
            run(&[tmp.path().to_path_buf()], &apps, &WalkLimits::balanced())
                .games
                .is_empty()
        );

        let with_hidden = WalkLimits {
            hidden: true,
            ..WalkLimits::balanced()
        };

        assert_eq!(
            run(&[tmp.path().to_path_buf()], &apps, &with_hidden)
                .games
                .len(),
            1
        );
    }

    /// Two ticked roots that nest must not report the same game twice.
    #[test]
    fn overlapping_roots_report_each_game_once() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        write(tmp.path(), "Games/gtav/GTA5.exe", "x");

        let report = run(
            &[tmp.path().to_path_buf(), tmp.path().join("Games")],
            &apps,
            &WalkLimits::balanced(),
        );

        assert_eq!(report.games.len(), 1, "{:?}", report.games);
    }

    #[test]
    fn an_unreadable_root_is_reported_rather_than_dropped() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        let missing = tmp.path().join("not-here");

        let report = run(
            std::slice::from_ref(&missing),
            &apps,
            &WalkLimits::balanced(),
        );

        assert_eq!(report.unreadable.len(), 1);
        assert_eq!(report.unreadable[0].0, missing.display().to_string());
    }

    #[test]
    fn limits_are_clamped_to_their_ceilings() {
        let absurd = WalkLimits {
            depth: 9_999,
            max_dirs: usize::MAX,
            budget_secs: 86_400,
            hidden: false,
        };

        let clamped = absurd.clamped();

        assert_eq!(clamped.depth, WalkLimits::MAX_DEPTH);
        assert_eq!(clamped.max_dirs, WalkLimits::MAX_DIRS);
        assert_eq!(clamped.budget_secs, WalkLimits::MAX_BUDGET_SECS);
    }

    /// A matched folder is not descended into — that is where the file count is.
    #[test]
    fn a_matched_folder_is_not_walked_further() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let plugin_dir = tempfile::tempdir().expect("tempdir");
        let apps = plugins(plugin_dir.path());

        write(tmp.path(), "gtav/GTA5.exe", "x");

        for i in 0..40 {
            std::fs::create_dir_all(tmp.path().join(format!("gtav/assets{i}"))).expect("mkdir");
        }

        let report = run(&[tmp.path().to_path_buf()], &apps, &WalkLimits::balanced());

        assert_eq!(report.games.len(), 1);
        // The root and the game folder, and nothing under it.
        assert_eq!(report.dirs_visited, 2, "descended into the game's own tree");
    }
}
