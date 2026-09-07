//! **Editing a game's own settings files**, from inside the app.
//!
//! Every mod manager studied for this has a config editor, and every one of
//! them exists for the same moment: a mod loader wrote `BepInEx/config/
//! com.author.mod.cfg` the first time the game ran, one value in it is wrong,
//! and changing it currently means leaving the manager, finding the folder and
//! opening a text editor.
//!
//! WHAT DECIDES WHICH FILES ARE OFFERED
//! -----------------------------------
//! The GAME does, through `plugins/app/<slug>/config.json`. That is the same
//! rule the rest of the app-plugin system follows — "where do this game's mods
//! go" is not knowledge the app has, and neither is "where does its loader keep
//! its settings". A game with no `config.json` has no config editor here, which
//! is the honest answer: a heuristic that swept the game folder for `*.cfg`
//! would find a hundred files a player must not touch, and offering somebody
//! their save file in a text editor is worse than offering nothing.
//!
//! WHY THIS STILL GOES THROUGH THE JAIL
//! -----------------------------------
//! Not because the process lacks permission — it plainly has it; the deployment
//! engine writes into game folders and the plugin executor unpacks archives
//! into them. The jail is about **which of those paths the webview can name**,
//! which is the rule at the top of `commands/mod.rs` and the reason a mod
//! description rendered next to `invoke` is a nuisance rather than a problem.
//!
//! So the shape is the one the rest of the system already uses: a declared
//! location compiles to an [`FsGrant`], every path resolves through
//! [`crate::plugins::jail`], and the editor cannot reach a file the game did
//! not declare. What that costs is nothing anybody wanted — a config editor
//! that could open `/etc/shadow` is not a better config editor.
//!
//! [`FsGrant`]: crate::plugins::manifest::FsGrant
//!
//! WHAT IS BOUNDED, AND WHY EACH ONE
//! --------------------------------
//! | Bound | The case it exists for |
//! | --- | --- |
//! | Per-file bytes | A game keeping a binary cache beside its settings, read into a webview as a string |
//! | File count | A location whose path resolves to a whole drive |
//! | Recursion depth | A loader that writes one folder per mod, per world |
//! | Text only | A file whose bytes are not UTF-8 is not something a text box can round-trip |
//!
//! **A write REPLACES the file and keeps a backup.** A config editor that
//! corrupts a working configuration with no way back is one people use once.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::plugins::apps::{ConfigLocation, ConfigSpec, MAX_CONFIG_DEPTH, MAX_CONFIG_FILES};
use crate::plugins::jail::Jail;
use crate::plugins::manifest::{FsRoot, PathRef};

/// One editable file, as the list shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFile {
    /// The location's label, so the UI can group without a second lookup.
    pub group: String,
    /// Which root it lives under. Half of the address the caller sends back.
    pub root: FsRoot,
    /// Path relative to that ROOT — not to the location — because that is what
    /// a [`PathRef`] means and what the jail resolves. Sending a
    /// location-relative path would need the location's own path added back
    /// somewhere, which is a second place to get it wrong.
    pub path: String,
    /// The last component, for display.
    pub name: String,
    pub size: u64,
    /// Milliseconds since the epoch, or null when the platform declines to say.
    pub modified_ms: Option<i64>,
    /// False when the file is too large, or is not text. Listed anyway, because
    /// "it is not here" and "it is here and cannot be edited" are different
    /// answers and only one of them is worth going to look for.
    pub editable: bool,
    /// Why not, when `editable` is false.
    pub reason: Option<String>,
}

/// Everything the editor may open for one game.
///
/// Bounded overall rather than per location: the cap exists to stop one absurd
/// location producing an unusable list, and a per-location cap would let
/// thirty-two of them do it together.
pub fn list(spec: &ConfigSpec, jail: &Jail) -> Vec<ConfigFile> {
    let mut out = Vec::new();
    let max_bytes = spec.max_bytes();

    for location in &spec.locations {
        if out.len() >= MAX_CONFIG_FILES {
            break;
        }

        /*
         * Resolved through the jail even though we built the jail from these
         * same locations. It costs one call and it means the one place a path
         * becomes a real directory is the one place that checks it — a later
         * change that let a location come from somewhere else would be caught
         * here rather than trusted.
         */
        let Ok(base) = jail.resolve(
            &PathRef {
                root: location.root,
                path: location.path.clone(),
            },
            false,
        ) else {
            continue;
        };

        if !base.is_dir() {
            // A location is always a folder — see `ConfigLocation::path`. One
            // that has not been created yet is a game that has not been run
            // yet, which is a normal state and not an error.
            continue;
        }

        walk(&base, &base, location, 0, max_bytes, &mut out);
    }

    out.truncate(MAX_CONFIG_FILES);
    out.sort_by(|a, b| {
        a.group
            .cmp(&b.group)
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
    });

    out
}

fn walk(
    dir: &Path,
    base: &Path,
    location: &ConfigLocation,
    depth: usize,
    max_bytes: u64,
    out: &mut Vec<ConfigFile>,
) {
    if out.len() >= MAX_CONFIG_FILES {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if out.len() >= MAX_CONFIG_FILES {
            return;
        }

        let path = entry.path();

        /*
         * `file_type`, which does not follow the link. A game folder with a
         * symlink in it pointing back at itself is unusual and a walk that
         * followed one would not terminate; more importantly a link pointing
         * OUT of the jail would be resolved by `read_dir` and never re-checked,
         * because the containment check happened on the location, not here.
         */
        let Ok(kind) = entry.file_type() else {
            continue;
        };

        if kind.is_symlink() {
            continue;
        }

        if kind.is_dir() {
            if location.recursive && depth < MAX_CONFIG_DEPTH {
                walk(&path, base, location, depth + 1, max_bytes, out);
            }

            continue;
        }

        let name = entry.file_name();
        let name = name.to_string_lossy();

        if !location.offers(&name) {
            continue;
        }

        // Relative to the ROOT, which is the location's own path plus the part
        // below it — see `ConfigFile::path`.
        let Ok(below) = path.strip_prefix(base) else {
            continue;
        };

        let relative = if location.path.is_empty() {
            below.to_string_lossy().replace('\\', "/")
        } else {
            format!(
                "{}/{}",
                location.path.trim_end_matches('/'),
                below.to_string_lossy().replace('\\', "/")
            )
        };

        if let Some(file) = describe(&path, location, &relative, max_bytes) {
            out.push(file);
        }
    }
}

fn describe(
    path: &Path,
    location: &ConfigLocation,
    relative: &str,
    max_bytes: u64,
) -> Option<ConfigFile> {
    let meta = std::fs::metadata(path).ok()?;
    let size = meta.len();

    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);

    let (editable, reason) = if size > max_bytes {
        (
            false,
            Some(format!(
                "Too large to edit here ({} KB). Open it in a text editor.",
                size / 1024
            )),
        )
    } else {
        (true, None)
    };

    Some(ConfigFile {
        group: location.label.clone(),
        root: location.root,
        path: relative.to_string(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| relative.to_string()),
        size,
        modified_ms,
        editable,
        reason,
    })
}

/// Which location, if any, offers this exact path.
///
/// **The jail is the outer bound and this is the inner one, and both are
/// needed.** A location naming the game's root directory — which
/// `{ path: "", files: ["options.txt"] }` legitimately does — grants the jail
/// the whole game folder, so the jail alone would happily resolve
/// `saves/world.dat`. This re-runs the spec's own matching rules against the
/// requested path, so the only files that can be read or written are the ones
/// the listing would have shown.
///
/// Checking it here rather than only in [`list`] is the whole point: a caller
/// names a path, not a row, and "it was in the list a moment ago" is not a
/// property of the string that arrives.
fn locate<'a>(spec: &'a ConfigSpec, root: FsRoot, path: &str) -> Option<&'a ConfigLocation> {
    let normalised = path.replace('\\', "/");
    let name = normalised.rsplit('/').next()?;

    if name.is_empty() {
        return None;
    }

    spec.locations.iter().find(|location| {
        if location.root != root {
            return false;
        }

        let prefix = location.path.trim_matches('/');

        // The part of the path below the location's own folder.
        let below = if prefix.is_empty() {
            Some(normalised.as_str())
        } else {
            normalised
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('/'))
        };

        let Some(below) = below else {
            return false;
        };

        let depth = below.matches('/').count();

        // A non-recursive location offers what is directly in it, and a
        // recursive one is still bounded — the same ceiling the walk uses, so
        // a path this permits is one the listing could have produced.
        if depth > 0 && (!location.recursive || depth > MAX_CONFIG_DEPTH) {
            return false;
        }

        location.offers(name)
    })
}

/// Read one file's text.
///
/// **UTF-8 or nothing.** A file whose bytes are not text cannot survive a round
/// trip through a text box: the webview would receive replacement characters
/// and write them back, which is a config editor that silently destroys the
/// file it was opened to fix.
pub fn read(jail: &Jail, spec: &ConfigSpec, root: FsRoot, path: &str) -> AppResult<String> {
    if locate(spec, root, path).is_none() {
        return Err(AppError::jail(format!(
            "'{path}' is not a settings file this game offers for editing."
        )));
    }

    let resolved = jail.resolve(
        &PathRef {
            root,
            path: path.to_string(),
        },
        false,
    )?;

    let meta = std::fs::metadata(&resolved)
        .map_err(|e| AppError::invalid(format!("That file could not be opened: {e}")))?;

    if !meta.is_file() {
        return Err(AppError::invalid("That is not a file."));
    }

    if meta.len() > spec.max_bytes() {
        return Err(AppError::invalid(
            "That file is too large to edit here. Open it in a text editor.",
        ));
    }

    let bytes = std::fs::read(&resolved)
        .map_err(|e| AppError::invalid(format!("That file could not be read: {e}")))?;

    String::from_utf8(bytes).map_err(|_| {
        AppError::invalid(
            "That file is not text, so it cannot be edited here without corrupting it.",
        )
    })
}

/// What one save did.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteOutcome {
    pub bytes: u64,
    /// Where the previous contents went, when there were any.
    pub backup: Option<String>,
}

/// Replace one file's contents, keeping the previous version.
///
/// **The backup is not optional.** A config editor that corrupts a working
/// configuration with no way back is one people use once and then go back to a
/// text editor — and the file being replaced is frequently the only copy of
/// settings somebody spent an evening on.
///
/// **Written to a temporary file and renamed.** A write interrupted halfway —
/// the app closing, the machine losing power — otherwise leaves a truncated
/// config, which for most loaders means a game that will not start. The rename
/// is atomic on every platform this ships to.
pub fn write(
    jail: &Jail,
    spec: &ConfigSpec,
    root: FsRoot,
    path: &str,
    contents: &str,
    backup_dir: &Path,
) -> AppResult<WriteOutcome> {
    if locate(spec, root, path).is_none() {
        return Err(AppError::jail(format!(
            "'{path}' is not a settings file this game offers for editing."
        )));
    }

    if contents.len() as u64 > spec.max_bytes() {
        return Err(AppError::invalid(
            "That is larger than this game allows a settings file to be.",
        ));
    }

    let resolved = jail.resolve(
        &PathRef {
            root,
            path: path.to_string(),
        },
        true,
    )?;

    if resolved.exists() && !resolved.is_file() {
        return Err(AppError::invalid("That is not a file."));
    }

    let backup = if resolved.is_file() {
        Some(keep_backup(&resolved, path, backup_dir)?)
    } else {
        None
    };

    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::invalid(format!("Could not prepare the folder: {e}")))?;
    }

    let temp = resolved.with_extension(format!(
        "{}.tmcsave",
        resolved
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));

    std::fs::write(&temp, contents.as_bytes())
        .map_err(|e| AppError::invalid(format!("Could not write the file: {e}")))?;

    std::fs::rename(&temp, &resolved).map_err(|e| {
        // The temporary file is removed on a failed rename, so a failure does
        // not leave a `.tmcsave` beside every config somebody tried to save.
        let _ = std::fs::remove_file(&temp);

        AppError::invalid(format!("Could not save the file: {e}"))
    })?;

    Ok(WriteOutcome {
        bytes: contents.len() as u64,
        backup: backup.map(|p| p.to_string_lossy().into_owned()),
    })
}

/// Copy the current contents somewhere recoverable.
///
/// Under the app's own backup directory rather than beside the file: a `.bak`
/// next to a config is a file the game's own loader may try to parse, and one
/// mod loader studied for this does exactly that.
fn keep_backup(source: &Path, relative: &str, backup_dir: &Path) -> AppResult<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let flat = relative
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();

    let dir = backup_dir.join("config");

    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::invalid(format!("Could not prepare the backup folder: {e}")))?;

    let target = dir.join(format!("{stamp}-{flat}"));

    std::fs::copy(source, &target)
        .map_err(|e| AppError::invalid(format!("Could not back the file up: {e}")))?;

    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use crate::plugins::apps::AppPluginFile;

    /// A game declaring one flat config folder and one single file.
    const CONFIG_JSON: &str = r#"{
        "manifestVersion": 1,
        "label": "Test config",
        "config": {
            "locations": [
                {
                    "label": "Mod settings",
                    "root": "gameDir",
                    "path": "BepInEx/config",
                    "extensions": ["cfg", "toml"],
                    "recursive": true
                },
                {
                    "label": "Game options",
                    "root": "gameDir",
                    "path": "",
                    "files": ["options.txt"]
                }
            ]
        }
    }"#;

    struct Fixture {
        _game: tempfile::TempDir,
        _backups: tempfile::TempDir,
        game: PathBuf,
        backups: PathBuf,
        file: AppPluginFile,
        jail: Jail,
    }

    /// Not named `write` — that is this module's own function, and a helper
    /// shadowing it inside the test module makes every call to the real one a
    /// confusing arity error rather than a test failure.
    fn put(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);

        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    fn fixture() -> Fixture {
        let plugin_dir = tempfile::tempdir().expect("tempdir");

        put(plugin_dir.path(), "app/testgame/config.json", CONFIG_JSON);

        let plugins = crate::plugins::apps::AppPlugins::load(plugin_dir.path());

        assert!(
            plugins.errors().is_empty(),
            "config.json did not load: {:?}",
            plugins.errors()
        );

        let file = plugins
            .config_file("testgame")
            .expect("a config rule")
            .clone();

        let game = tempfile::tempdir().expect("tempdir");
        let backups = tempfile::tempdir().expect("tempdir");

        let mut available: HashMap<&'static str, PathBuf> = HashMap::new();

        available.insert("gameDir", game.path().to_path_buf());

        let jail = Jail::build(&file.as_manifest(), &available).expect("jail");

        Fixture {
            game: game.path().to_path_buf(),
            backups: backups.path().to_path_buf(),
            _game: game,
            _backups: backups,
            file,
            jail,
        }
    }

    fn spec(f: &Fixture) -> &ConfigSpec {
        f.file.config.as_ref().expect("config")
    }

    #[test]
    fn a_declared_folder_is_listed_and_an_undeclared_one_is_not() {
        let f = fixture();

        put(&f.game, "BepInEx/config/mod.cfg", "a = 1");
        put(&f.game, "BepInEx/config/nested/other.toml", "b = 2");
        put(&f.game, "options.txt", "fov:90");
        // Neither declared nor matching an extension.
        put(&f.game, "saves/world.dat", "binary-ish");
        put(&f.game, "BepInEx/config/notes.md", "# hello");

        let found = list(spec(&f), &f.jail);
        let paths: Vec<&str> = found.iter().map(|c| c.path.as_str()).collect();

        assert!(paths.contains(&"BepInEx/config/mod.cfg"), "{paths:?}");
        assert!(
            paths.contains(&"BepInEx/config/nested/other.toml"),
            "{paths:?}"
        );
        assert!(paths.contains(&"options.txt"), "{paths:?}");

        assert!(!paths.iter().any(|p| p.contains("saves")), "{paths:?}");
        assert!(!paths.iter().any(|p| p.ends_with(".md")), "{paths:?}");
    }

    /// The rule the whole module rests on.
    ///
    /// The webview names a path, and the only paths it may name are ones the
    /// GAME declared. That is not about the process's permissions — it plainly
    /// has them — it is about a mod description rendered next to `invoke`.
    #[test]
    fn a_path_outside_a_declared_location_is_refused() {
        let f = fixture();

        put(&f.game, "saves/world.dat", "mine");

        for bad in [
            "saves/world.dat",
            "../../../etc/passwd",
            "BepInEx/config/../../saves/world.dat",
            "/etc/passwd",
        ] {
            assert!(
                read(&f.jail, spec(&f), FsRoot::GameDir, bad).is_err(),
                "{bad} should be refused"
            );

            assert!(
                write(&f.jail, spec(&f), FsRoot::GameDir, bad, "pwned", &f.backups).is_err(),
                "{bad} should be refused for writing"
            );
        }

        // And the file it tried to reach is untouched.
        assert_eq!(
            std::fs::read_to_string(f.game.join("saves/world.dat")).expect("read"),
            "mine"
        );
    }

    #[test]
    fn a_file_round_trips_and_keeps_a_backup() {
        let f = fixture();

        put(&f.game, "BepInEx/config/mod.cfg", "value = 1");

        let before =
            read(&f.jail, spec(&f), FsRoot::GameDir, "BepInEx/config/mod.cfg").expect("read");

        assert_eq!(before, "value = 1");

        let outcome = write(
            &f.jail,
            spec(&f),
            FsRoot::GameDir,
            "BepInEx/config/mod.cfg",
            "value = 2",
            &f.backups,
        )
        .expect("write");

        let after =
            read(&f.jail, spec(&f), FsRoot::GameDir, "BepInEx/config/mod.cfg").expect("read");

        assert_eq!(after, "value = 2");

        let backup = outcome.backup.expect("a backup");

        assert_eq!(
            std::fs::read_to_string(&backup).expect("backup"),
            "value = 1",
            "the backup must hold what was replaced"
        );
    }

    /// A backup beside the file is one the game's own loader may try to parse.
    #[test]
    fn the_backup_is_not_left_in_the_game_folder() {
        let f = fixture();

        put(&f.game, "BepInEx/config/mod.cfg", "value = 1");

        write(
            &f.jail,
            spec(&f),
            FsRoot::GameDir,
            "BepInEx/config/mod.cfg",
            "value = 2",
            &f.backups,
        )
        .expect("write");

        let left: Vec<String> = std::fs::read_dir(f.game.join("BepInEx/config"))
            .expect("read_dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();

        assert_eq!(left, vec!["mod.cfg".to_string()], "{left:?}");
    }

    /// A text box cannot round-trip bytes that are not text.
    ///
    /// Reading one as lossy UTF-8 and writing it back replaces every invalid
    /// byte with U+FFFD, which is a config editor that destroys the file it was
    /// opened to fix.
    #[test]
    fn a_file_that_is_not_text_is_refused_rather_than_mangled() {
        let f = fixture();

        std::fs::create_dir_all(f.game.join("BepInEx/config")).expect("mkdir");
        std::fs::write(
            f.game.join("BepInEx/config/binary.cfg"),
            [0xff, 0xfe, 0x00, 0x01],
        )
        .expect("write");

        let err = read(
            &f.jail,
            spec(&f),
            FsRoot::GameDir,
            "BepInEx/config/binary.cfg",
        )
        .expect_err("refused");

        assert!(err.to_string().contains("not text"), "{err}");
    }

    #[test]
    fn a_file_over_the_cap_is_listed_but_not_editable() {
        let f = fixture();

        let big = "x".repeat((crate::plugins::apps::DEFAULT_CONFIG_MAX_BYTES + 1) as usize);

        put(&f.game, "BepInEx/config/huge.cfg", &big);

        let found = list(spec(&f), &f.jail);

        let row = found
            .iter()
            .find(|c| c.path == "BepInEx/config/huge.cfg")
            .expect("listed");

        assert!(!row.editable);
        assert!(row.reason.is_some());

        // And reading it is refused rather than pulling it into the webview.
        assert!(read(&f.jail, spec(&f), FsRoot::GameDir, &row.path).is_err());
    }

    #[test]
    fn a_symlink_is_not_followed_out_of_the_jail() {
        let f = fixture();

        put(&f.game, "BepInEx/config/real.cfg", "ok");
        put(&f.game, "saves/secret.cfg", "not yours");

        #[cfg(unix)]
        std::os::unix::fs::symlink(f.game.join("saves"), f.game.join("BepInEx/config/linked"))
            .expect("symlink");

        let found = list(spec(&f), &f.jail);

        assert!(
            !found.iter().any(|c| c.path.contains("secret")),
            "a symlink was followed: {found:?}"
        );
    }

    /// Grants are derived from the locations, so an author cannot forget one.
    #[test]
    fn every_declared_location_is_reachable() {
        let f = fixture();

        put(&f.game, "BepInEx/config/mod.cfg", "a");
        put(&f.game, "options.txt", "b");

        for path in ["BepInEx/config/mod.cfg", "options.txt"] {
            assert!(
                read(&f.jail, spec(&f), FsRoot::GameDir, path).is_ok(),
                "{path} should be reachable"
            );
        }
    }

    /// The inner bound, and why the jail alone is not enough.
    ///
    /// The "Game options" location names the game's ROOT so that
    /// `options.txt` can be offered, which means the jail grants the whole
    /// game folder. Without the spec's own rules re-run on every read and
    /// write, `saves/world.dat` would resolve perfectly well.
    #[test]
    fn a_broad_location_does_not_widen_what_can_be_opened() {
        let f = fixture();

        put(&f.game, "options.txt", "fov:90");
        put(&f.game, "saves/world.dat", "mine");
        put(&f.game, "steam_api64.dll", "not a config");

        // Offered.
        assert!(read(&f.jail, spec(&f), FsRoot::GameDir, "options.txt").is_ok());

        // In the same granted root, and not offered.
        for hidden in ["saves/world.dat", "steam_api64.dll"] {
            let err =
                read(&f.jail, spec(&f), FsRoot::GameDir, hidden).expect_err("should be refused");

            assert!(
                err.to_string().contains("offers for editing"),
                "{hidden}: {err}"
            );

            assert!(
                write(&f.jail, spec(&f), FsRoot::GameDir, hidden, "x", &f.backups).is_err(),
                "{hidden} should be refused for writing"
            );
        }

        assert_eq!(
            std::fs::read_to_string(f.game.join("saves/world.dat")).expect("read"),
            "mine"
        );
    }

    /// A location naming a single FILE would have `Jail::build` create a
    /// DIRECTORY there, destroying the settings file it was meant to expose.
    /// Locations are folders; the file is named in `files`.
    #[test]
    fn a_named_file_survives_the_jail_being_built() {
        let f = fixture();

        assert!(
            !f.game.join("options.txt").is_dir(),
            "the grant turned the file into a folder"
        );

        put(&f.game, "options.txt", "fov:90");

        // And a second jail over the same folder does not clobber it either.
        let mut available: HashMap<&'static str, PathBuf> = HashMap::new();

        available.insert("gameDir", f.game.clone());

        let again = Jail::build(&f.file.as_manifest(), &available).expect("jail");

        assert_eq!(
            read(&again, spec(&f), FsRoot::GameDir, "options.txt").expect("read"),
            "fov:90"
        );
    }

    #[test]
    fn a_config_location_naming_a_traversal_is_refused_at_load() {
        let plugin_dir = tempfile::tempdir().expect("tempdir");

        put(
            plugin_dir.path(),
            "app/bad/config.json",
            r#"{
                "manifestVersion": 1,
                "config": {
                    "locations": [
                        { "label": "Escape", "root": "gameDir", "path": "../../etc" }
                    ]
                }
            }"#,
        );

        let plugins = crate::plugins::apps::AppPlugins::load(plugin_dir.path());

        assert!(plugins.config_spec("bad").is_none());
        assert!(
            !plugins.errors().is_empty(),
            "the load error was not recorded"
        );
    }
}
