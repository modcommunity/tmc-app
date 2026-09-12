//! **Games TMC publishes, installed on this machine.**
//!
//! Everything else in this crate manages somebody else's game: `detect` finds
//! where Steam put it, `deploy` puts mods into it, `launch` starts the copy that
//! is already there. This module is the one place the app is the distributor —
//! it downloads a build, unpacks it under the app's own data directory, keeps it
//! current, and starts it.
//!
//! ```text
//!   /apps/:id/build?platform=…   ── the site resolves ONE build for THIS machine
//!            │
//!            ▼
//!   download::DownloadManager    ── the same queue, the same limits, the same
//!            │                      pause button as every other download
//!            ▼
//!   plugins::steps::extract      ── the same zip-slip guard and expansion cap
//!            │
//!            ▼
//!   library::db native_game      ── the only record that these files are ours
//!            │
//!            ▼
//!   launch::LaunchPlan → spawn   ── a real process, with a real play session
//! ```
//!
//! NOTHING HERE IS NEW MACHINERY, AND THAT IS THE DESIGN
//! ----------------------------------------------------
//! A game install is a download, an extraction, a row and a launch, and this app
//! already had one of each — with the bounds, the audit entries and the failure
//! modes worked out. Writing a second downloader would have meant a second place
//! for a bandwidth limit to be ignored; writing a second unpacker would have
//! meant a second zip-slip guard, and the one that was wrong would be the one
//! nobody was reading. What this module contributes is the sequencing and the
//! refusals, not the mechanisms.
//!
//! WHY A GAME IS NOT A SUBSCRIPTION
//! -------------------------------
//! `subscription` is a MIRROR the full sync rewrites: anything the server did
//! not send is deleted from it, because that is how an unsubscribe on another
//! device reaches this one. A game on this disk is a fact about this disk — the
//! account knows which games exist and cannot know which of them somebody
//! unpacked onto a laptop — so it lives in its own table for the same reason
//! `local_mod` does. A sync must not be able to forget an install.
//!
//! WHY THE WEBVIEW STILL NAMES NO PATHS
//! -----------------------------------
//! Every path here is produced on this side: the install root comes from Tauri's
//! path resolver, the version directory from the build the SITE resolved, and
//! the entry from that build's own declaration put through the same lexical
//! checks a plugin's `PathRef` gets. The IPC surface takes an app id and, for a
//! launch, a server id. There is no `game_install(url)` and no
//! `game_launch(path)`.
//!
//! WHAT AN INSTALL IS NOT
//! ---------------------
//! It is not a sandbox, and the two must not be confused. A sandbox deploys mods
//! into a game folder that somebody else's launcher owns; this owns the folder
//! outright and has no mods in it at all. A game installed here that later grows
//! a modding story gets a `sandbox.json` like any other game and the sandbox
//! engine works on it unchanged — `dir` is a game directory like any other.

pub mod build;
pub mod platform;
pub mod updates;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::api::{ApiClient, Method};
use crate::download::{DownloadManager, DownloadRequest};
use crate::error::{AppError, AppResult};
use crate::launch::LaunchPlan;
use crate::library::db::{InstalledGame, LibraryDb};
use crate::logging::Audit;
use crate::{audit, plugins};

pub use build::{build_args, BuildFormat, LaunchContext, NativeBuild};
pub use platform::{current as current_platform, BuildPlatform};

/// Segments the install root may never gain from a slug.
///
/// The directory name comes from `App.url`, which is a value the SITE chose —
/// but "the server would never send that" is a claim about a server, not a
/// property of the string in hand, and this string becomes a path. Anything
/// that is not a plain name falls back to the numeric id, which cannot be
/// anything else.
fn dir_name(app_id: i64, slug: Option<&str>) -> String {
    let slug = slug.unwrap_or_default().trim();

    let usable = !slug.is_empty()
        && slug.len() <= 64
        && slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');

    if usable {
        slug.to_ascii_lowercase()
    } else {
        format!("app-{app_id}")
    }
}

/// A version string, as a directory name.
///
/// Same rule, and it matters more: `version` is compared numerically elsewhere
/// but is stored as free text, so a build published as `../../etc` would
/// otherwise name a directory this module then deletes on the next update.
fn version_dir(version: &str) -> AppResult<String> {
    let trimmed = version.trim();

    let usable = !trimmed.is_empty()
        && trimmed.len() <= 64
        && trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_' || b == b'+');

    if usable {
        Ok(trimmed.to_string())
    } else {
        Err(AppError::invalid(
            "That build's version is not one the app will make a folder for.",
        ))
    }
}

/// One installed game plus whatever the site last said about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameStatus {
    #[serde(flatten)]
    pub installed: InstalledGame,
    /// The version available, when a check has run and found one.
    pub available: Option<String>,
    /// Whether `available` is actually newer — computed here so the card, the
    /// update pass and the Library cannot disagree about what "an update" is.
    pub update_available: bool,
    pub notes: Option<String>,
}

/// The installer, the updater and the launcher for TMC's own games.
pub struct Games {
    api: Arc<ApiClient>,
    downloads: DownloadManager,
    db: Arc<LibraryDb>,
    pub(crate) audit: Arc<Audit>,
    /// `<app data>/games`. In DATA rather than cache, for the reason staging is:
    /// an OS that cleared this would silently uninstall somebody's games.
    root: PathBuf,
    /// Where a build is downloaded to before it is unpacked. In CACHE, because
    /// once it is unpacked the archive is dead weight — and a machine that
    /// cleared it mid-download costs a retry rather than an install.
    scratch: PathBuf,
    /// This app's own version, for a build's `minClientVersion`.
    client_version: String,
}

impl Games {
    pub fn new(
        api: Arc<ApiClient>,
        downloads: DownloadManager,
        db: Arc<LibraryDb>,
        audit: Arc<Audit>,
        root: PathBuf,
        scratch: PathBuf,
        client_version: String,
    ) -> Self {
        Self {
            api,
            downloads,
            db,
            audit,
            root,
            scratch,
            client_version,
        }
    }

    /// This machine's build target, or `None` for one nothing is published for.
    pub fn platform(&self) -> Option<BuildPlatform> {
        current_platform()
    }

    /// What the site has for this machine, or `None` for "nothing to install".
    ///
    /// `null` is the endpoint's whole refusal vocabulary — the app is hidden,
    /// the build is disabled, nothing is published for this platform — and it is
    /// kept as one answer here for the same reason it is one answer there: none
    /// of them changes what the device does next.
    pub async fn resolve(&self, app_id: i64) -> AppResult<Option<NativeBuild>> {
        let Some(platform) = self.platform() else {
            return Ok(None);
        };

        let answer = self
            .api
            .request(
                Method::GET,
                &format!("/apps/{app_id}/build?platform={}", platform.as_str()),
                None,
                false,
            )
            .await?;

        if answer.is_null() {
            return Ok(None);
        }

        let build = serde_json::from_value::<NativeBuild>(answer)
            .map_err(|e| AppError::internal(format!("apps/{app_id}/build: {e}")))?;

        /*
         * The platform is checked against what we ASKED for rather than
         * trusted. A row for the wrong architecture is a publishing mistake on
         * the other side, and installing it would produce a game that does not
         * start with nothing on screen to explain why.
         */
        if build.platform != platform {
            return Ok(None);
        }

        Ok(Some(build))
    }

    pub fn installed(&self) -> AppResult<Vec<InstalledGame>> {
        self.db.games()
    }

    pub fn get(&self, app_id: i64) -> AppResult<Option<InstalledGame>> {
        self.db.game(app_id)
    }

    /// Install or update one game.
    ///
    /// The same function for both, deliberately. An update is an install whose
    /// row already exists, and every step below is identical — a separate
    /// `update` would be a second copy of the download, the extraction and the
    /// verification, and the copy that drifted would be the one that runs less
    /// often.
    ///
    /// **The previous version is not removed until the new one is in place.**
    /// The new build lands in its own version directory and the row is only
    /// repointed once the entry is known to exist, so a failed update leaves a
    /// working game rather than a directory somebody has to reinstall into.
    pub async fn install(
        &self,
        app_id: i64,
        name: &str,
        slug: Option<&str>,
    ) -> AppResult<InstalledGame> {
        let Some(platform) = self.platform() else {
            return Err(AppError::invalid(
                "The app does not publish games for this kind of machine.",
            ));
        };

        if !platform.installable() {
            return Err(AppError::invalid(
                "Games are installed through this platform's own store, not by the app.",
            ));
        }

        let Some(build) = self.resolve(app_id).await? else {
            return Err(AppError::invalid(
                "There is no build of that game for this machine.",
            ));
        };

        build.check(&self.client_version)?;

        let version = version_dir(&build.version)?;
        let game_dir = self.root.join(dir_name(app_id, slug));
        let dest = game_dir.join(&version);

        // A retry after a failure would otherwise unpack on top of a partial
        // tree, where a file the new archive does not contain survives from the
        // last attempt and gets launched.
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }

        std::fs::create_dir_all(&dest)?;
        std::fs::create_dir_all(&self.scratch)?;

        let archive = self.scratch.join(format!(
            "game-{app_id}-{version}.{}",
            build.format.extension()
        ));

        audit!(
            self.audit,
            Info,
            App,
            "game.install.start",
            format!("{name} {} for {}", build.version, platform.as_str()),
            data = audit_data([("appId", app_id.into()), ("url", build.url.clone().into()),])
        );

        /*
         * THROUGH THE QUEUE, not a bare fetch.
         *
         * A game build is the largest thing this app downloads, so it is the
         * one that most needs the bandwidth limit, the pause button and the
         * resume — and the one a user is most likely to be watching. A
         * deterministic id means a retry is the same row rather than a second
         * writer for one file.
         */
        self.downloads
            .run_to_completion(DownloadRequest {
                id: format!("game:{app_id}:{version}"),
                url: build.url.clone(),
                dest: archive.clone(),
                label: format!("{name} {}", build.version),
                sha256: Some(build.sha256.clone()),
                size_hint: Some(build.size_bytes),
                priority: 0,
                limit_bps: None,
                meta: [
                    ("kind".to_string(), "game".to_string()),
                    ("appId".to_string(), app_id.to_string()),
                ]
                .into_iter()
                .collect(),
            })
            .await?;

        let outcome = self.unpack(&build, &archive, &dest);

        // The archive is dead weight once it is unpacked, and it is the largest
        // file the app ever writes. Removed on failure too: a retry downloads it
        // again, which is cheaper than a cache directory nobody empties.
        let _ = std::fs::remove_file(&archive);

        if let Err(error) = outcome {
            let _ = std::fs::remove_dir_all(&dest);

            return Err(error);
        }

        let entry = self.entry_path(&build, &dest)?;

        let now = crate::session::now_ms();

        let row = InstalledGame {
            app_id,
            slug: slug.map(str::to_string),
            name: name.to_string(),
            platform: platform.as_str().to_string(),
            version: build.version.clone(),
            dir: dest.to_string_lossy().to_string(),
            entry: entry.map(|e| e.to_string_lossy().to_string()),
            args: build.args.clone(),
            size_bytes: build.size_bytes as i64,
            installed_ms: now,
            updated_ms: now,
            // Inherited across an update, so turning it off stays off. A fresh
            // install starts on: somebody who asked the app to install a game
            // asked it to keep the game working.
            auto_update: self
                .db
                .game(app_id)?
                .map(|previous| previous.auto_update)
                .unwrap_or(true),
        };

        self.db.game_put(&row)?;

        // LAST, and only now that the row points somewhere else. Doing it before
        // the write would mean a failure in between left the row naming a
        // directory that had just been deleted.
        prune_old_versions(&game_dir, &version);

        audit!(
            self.audit,
            Info,
            App,
            "game.install.done",
            format!("{name} {}", build.version),
            data = audit_data([("appId", app_id.into()), ("dir", row.dir.clone().into()),])
        );

        Ok(row)
    }

    /// Put the downloaded artifact where it belongs.
    fn unpack(&self, build: &NativeBuild, archive: &Path, dest: &Path) -> AppResult<()> {
        unpack(build, archive, dest)
    }

    /// Where the executable ended up, checked rather than assumed.
    fn entry_path(&self, build: &NativeBuild, dest: &Path) -> AppResult<Option<PathBuf>> {
        entry_path(build, dest)
    }

    /// What to start, and with which arguments.
    ///
    /// Produces a plan and runs NOTHING, exactly as `launch::plan` does — the
    /// decision of how a process is started belongs to `spawn`, which is the
    /// only place in the app that starts one.
    pub fn plan(&self, row: &InstalledGame, ctx: &LaunchContext) -> AppResult<LaunchPlan> {
        plan(row, ctx)
    }
}

/// Put the downloaded artifact where it belongs.
fn unpack(build: &NativeBuild, archive: &Path, dest: &Path) -> AppResult<()> {
    if !build.format.is_archive() {
        /*
         * A single-file build keeps the name the URL gave it, so a game
         * whose executable is `Hungario.x86_64` is not installed as
         * `game-42-1.0.0.bin`. The name is sanitised the same way every
         * other untrusted path segment in this crate is.
         */
        let name = build
            .url
            .rsplit('/')
            .next()
            .and_then(|n| n.split(['?', '#']).next())
            .unwrap_or_default();

        let name = plugins::jail::join_relative(dest, name)
            .ok()
            .filter(|p| p.parent() == Some(dest))
            .unwrap_or_else(|| dest.join("game.bin"));

        std::fs::copy(archive, &name)?;

        return Ok(());
    }

    /*
     * ONE EXTRACTION IMPLEMENTATION for the whole app.
     *
     * `strip` is zero and `include` is empty: a build's `entry` is declared
     * relative to the archive as published, so stripping a wrapping
     * directory here would make that declaration wrong — and the publisher
     * is the one who can see what is in their own archive.
     */
    plugins::steps::extract_archive(archive, dest, 0, &[])?;

    Ok(())
}

/// Where the executable ended up, checked rather than assumed.
///
/// The single most valuable check in this module: an archive that unpacked
/// perfectly and does not contain the file it said it would is an install that
/// reports success and then cannot be started, with nothing on screen to say
/// why.
fn entry_path(build: &NativeBuild, dest: &Path) -> AppResult<Option<PathBuf>> {
    let Some(declared) = build
        .entry
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
    else {
        // `RAW` — the artifact is the executable and `unpack` named it, so
        // the directory's single file is the answer.
        let mut entries = std::fs::read_dir(dest)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect::<Vec<_>>();

        entries.retain(|p| p.is_file());

        let Some(only) = entries.pop().filter(|_| entries.is_empty()) else {
            return Err(AppError::internal("the build did not unpack to one file"));
        };

        mark_executable(&only)?;

        return Ok(only
            .strip_prefix(dest)
            .ok()
            .map(std::path::Path::to_path_buf));
    };

    /*
     * Through the jail's own lexical check, not `Path::join`.
     *
     * `entry` is a string from the API that becomes a path this process
     * executes, and `Path::join` with an absolute argument DISCARDS the
     * base — the single behaviour `join_relative` exists for. Nothing about
     * this value having come from our own server changes what `join` does
     * with `/usr/bin/sh`.
     */
    let full = plugins::jail::join_relative(dest, declared)?;

    if !full.is_file() {
        return Err(AppError::invalid(
            "That build unpacked without the file it says to start.",
        ));
    }

    mark_executable(&full)?;

    Ok(Some(PathBuf::from(declared)))
}

/// Delete every version directory but the one just installed.
///
/// Best-effort on purpose. A leftover directory costs disk; a failure here
/// aborting an install that has already succeeded costs the install.
fn prune_old_versions(game_dir: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(game_dir) else {
        return;
    };

    for entry in entries.filter_map(Result::ok) {
        if entry.file_name() == keep {
            continue;
        }

        if entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

impl Games {
    /// Remove a game's files and forget it.
    ///
    /// The FILES first and the row last, so a failure halfway leaves a row
    /// naming a partly-removed install — which the app can see and offer to
    /// finish — rather than a directory nothing knows about.
    pub fn uninstall(&self, app_id: i64) -> AppResult<()> {
        let Some(row) = self.db.game(app_id)? else {
            return Ok(());
        };

        let dir = PathBuf::from(&row.dir);

        /*
         * The version directory's PARENT is what gets removed, because that is
         * what the install created — and an interrupted update can have left a
         * sibling version beside it that the row does not name.
         *
         * Refused unless it is genuinely under our own root. `dir` came out of
         * this app's database, which is a file on the user's disk; a corrupted
         * or hand-edited row naming `/` must not be able to turn "uninstall a
         * game" into a recursive delete somewhere else.
         */
        let target = dir.parent().unwrap_or(&dir);

        if !target.starts_with(&self.root) || target == self.root {
            return Err(AppError::internal(
                "that game's folder is not one the app installed",
            ));
        }

        if target.exists() {
            std::fs::remove_dir_all(target)?;
        }

        self.db.game_delete(app_id)?;

        audit!(
            self.audit,
            Info,
            App,
            "game.uninstall",
            row.name.clone(),
            data = audit_data([("appId", app_id.into())])
        );

        Ok(())
    }

    pub fn set_auto_update(&self, app_id: i64, on: bool) -> AppResult<()> {
        self.db.game_set_auto_update(app_id, on)
    }
}

/// What to start, and with which arguments.
fn plan(row: &InstalledGame, ctx: &LaunchContext) -> AppResult<LaunchPlan> {
    let dir = PathBuf::from(&row.dir);

    let program = match row.entry.as_deref() {
        Some(entry) => plugins::jail::join_relative(&dir, entry)?,
        None => {
            return Err(AppError::internal(
                "that install does not record what to start",
            ))
        }
    };

    if !program.is_file() {
        return Err(AppError::invalid(
            "That game's files are missing. Reinstall it from the Library.",
        ));
    }

    Ok(LaunchPlan {
        rule: format!("tmc-game/{}", row.app_id),
        program: Some(program.to_string_lossy().to_string()),
        uri: None,
        args: build_args(&row.args, ctx),
        /*
         * The EXECUTABLE's directory, not the install root.
         *
         * A game loads its packs and its settings relative to where it was
         * started, and an export nested one directory deep inside its
         * archive — which is what zipping an export folder gives you —
         * would otherwise look for its own data one level up and find
         * nothing. The symptom is a game that starts to a black window.
         */
        cwd: program
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .or_else(|| Some(row.dir.clone())),
        env: Default::default(),
        hide_window: false,
        // A TMC game is not a sandbox and carries no virtual filesystem.
        vfs: None,
    })
}

/// Structured audit extras, as the log entry wants them.
///
/// A helper rather than `json!({…})` at each site because `LogEntry::data` is a
/// `serde_json::Map` and not a `Value` — the macro writes it straight into the
/// field, so a call site producing an object would have to unwrap it and a call
/// site producing anything else would not compile in a way that says so.
fn audit_data<const N: usize>(
    pairs: [(&str, serde_json::Value); N],
) -> serde_json::Map<String, serde_json::Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// Give a file the executable bit, on the platforms that have one.
///
/// Needed because a ZIP's stored Unix mode does not survive every packer, every
/// extractor or every filesystem — and a Godot export whose binary lands without
/// `+x` fails at launch with a permission error that reads like a broken
/// download. Windows has no such bit and needs none.
fn mark_executable(path: &Path) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = std::fs::metadata(path)?.permissions();

        perms.set_mode(perms.mode() | 0o755);

        std::fs::set_permissions(path, perms)?;
    }

    #[cfg(not(unix))]
    let _ = path;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::build::BuildFormat;
    use std::io::Write;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tmc-games-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));

        std::fs::create_dir_all(&dir).expect("temp dir");

        dir
    }

    /// A zip holding one executable and one data file, under a wrapping folder.
    ///
    /// The wrapper is there on purpose: it is what zipping an export folder
    /// gives you, and it is why `entry` is declared relative to the archive as
    /// published rather than stripped by this end.
    fn zip_with(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).expect("create zip");
        let mut writer = zip::ZipWriter::new(file);

        for (name, body) in entries {
            writer
                .start_file::<_, ()>(*name, zip::write::SimpleFileOptions::default())
                .expect("entry");
            writer.write_all(body).expect("write");
        }

        writer.finish().expect("finish");
    }

    fn build(entry: Option<&str>, args: &[&str]) -> NativeBuild {
        NativeBuild {
            app_id: 42,
            platform: BuildPlatform::LinuxX64,
            version: "1.0.0".into(),
            url: "https://cdn.example.com/game.zip".into(),
            sha256: "a".repeat(64),
            size_bytes: 512,
            format: BuildFormat::Zip,
            entry: entry.map(str::to_string),
            args: args.iter().map(|a| (*a).to_string()).collect(),
            min_client_version: None,
            notes: None,
        }
    }

    #[test]
    fn an_archive_unpacks_and_its_declared_entry_is_found() {
        let root = temp("unpack");
        let archive = root.join("game.zip");
        let dest = root.join("1.0.0");

        std::fs::create_dir_all(&dest).expect("dest");
        zip_with(
            &archive,
            &[
                ("game/game.x86_64", b"#!/bin/sh\nexit 0\n"),
                ("game/game.pck", b"data"),
            ],
        );

        let build = build(Some("game/game.x86_64"), &[]);

        unpack(&build, &archive, &dest).expect("unpack");

        let entry = entry_path(&build, &dest).expect("entry");

        assert_eq!(entry, Some(PathBuf::from("game/game.x86_64")));
        assert!(dest.join("game/game.pck").is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            // A Godot export whose binary lands without `+x` fails at launch
            // with a permission error that reads like a broken download.
            let mode = std::fs::metadata(dest.join("game/game.x86_64"))
                .expect("stat")
                .permissions()
                .mode();

            assert_eq!(mode & 0o111, 0o111, "the entry is executable");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The single most valuable check in the module: an archive that unpacked
    /// perfectly and does not hold the file it named is an install that reports
    /// success and then cannot be started.
    #[test]
    fn an_archive_without_the_file_it_names_is_refused() {
        let root = temp("missing");
        let archive = root.join("game.zip");
        let dest = root.join("1.0.0");

        std::fs::create_dir_all(&dest).expect("dest");
        zip_with(&archive, &[("game/readme.txt", b"nothing to run")]);

        let build = build(Some("game/game.x86_64"), &[]);

        unpack(&build, &archive, &dest).expect("unpack");

        assert!(entry_path(&build, &dest).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A build's `entry` is a string from the network that becomes a path this
    /// process executes, and `Path::join` with an absolute argument DISCARDS
    /// the base — which is the whole reason `join_relative` exists.
    #[test]
    fn an_entry_that_escapes_the_install_is_refused() {
        let root = temp("escape");
        let archive = root.join("game.zip");
        let dest = root.join("1.0.0");

        std::fs::create_dir_all(&dest).expect("dest");
        zip_with(&archive, &[("game.x86_64", b"x")]);

        for bad in ["../../../bin/sh", "/bin/sh", "game/../../../bin/sh"] {
            let build = build(Some(bad), &[]);

            unpack(&build, &archive, &dest).expect("unpack");

            assert!(entry_path(&build, &dest).is_err(), "{bad} must be refused");
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_launch_plan_names_the_executable_and_joins_a_server() {
        let root = temp("plan");
        let dest = root.join("1.0.0");

        std::fs::create_dir_all(dest.join("game")).expect("dirs");
        std::fs::write(dest.join("game/game.x86_64"), b"x").expect("binary");

        let row = InstalledGame {
            app_id: 42,
            slug: Some("hungario".into()),
            name: "Hungario".into(),
            platform: "LINUX_X64".into(),
            version: "1.0.0".into(),
            dir: dest.to_string_lossy().to_string(),
            entry: Some("game/game.x86_64".into()),
            args: vec![
                "--game".into(),
                "{app}".into(),
                "--connect".into(),
                "{host}:{port}".into(),
            ],
            size_bytes: 1,
            installed_ms: 0,
            updated_ms: 0,
            auto_update: true,
        };

        let joined = plan(
            &row,
            &LaunchContext {
                app_id: 42,
                app_slug: Some("hungario".into()),
                host: Some("play.example.com".into()),
                port: Some(6064),
                ..Default::default()
            },
        )
        .expect("plan");

        assert_eq!(
            joined.program.as_deref(),
            Some(dest.join("game/game.x86_64").to_string_lossy().as_ref())
        );
        assert_eq!(
            joined.args,
            vec!["--game", "hungario", "--connect", "play.example.com:6064"]
        );
        // The EXECUTABLE's directory, not the install root: an export nested
        // one level inside its archive looks for its own data relative to where
        // it was started.
        assert_eq!(
            joined.cwd.as_deref(),
            Some(dest.join("game").to_string_lossy().as_ref())
        );
        // Nothing here is a hand-off and nothing carries a virtual filesystem.
        assert!(joined.uri.is_none());
        assert!(joined.vfs.is_none());

        // With no server, the game starts at its own menu rather than dialling
        // an address that is not there.
        let alone = plan(
            &row,
            &LaunchContext {
                app_id: 42,
                app_slug: Some("hungario".into()),
                ..Default::default()
            },
        )
        .expect("plan");

        assert_eq!(alone.args, vec!["--game", "hungario"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_plan_for_files_that_are_gone_is_refused_rather_than_spawned() {
        let row = InstalledGame {
            app_id: 42,
            slug: None,
            name: "Gone".into(),
            platform: "LINUX_X64".into(),
            version: "1.0.0".into(),
            dir: "/nonexistent/tmc-games/1.0.0".into(),
            entry: Some("game".into()),
            args: vec![],
            size_bytes: 1,
            installed_ms: 0,
            updated_ms: 0,
            auto_update: true,
        };

        assert!(plan(&row, &LaunchContext::default()).is_err());
    }

    #[test]
    fn pruning_keeps_the_version_just_installed_and_nothing_else() {
        let game_dir = temp("prune");

        for version in ["1.0.0", "0.9.0", "0.8.0"] {
            std::fs::create_dir_all(game_dir.join(version)).expect("version dir");
        }

        prune_old_versions(&game_dir, "1.0.0");

        assert!(game_dir.join("1.0.0").is_dir());
        assert!(!game_dir.join("0.9.0").exists());
        assert!(!game_dir.join("0.8.0").exists());

        let _ = std::fs::remove_dir_all(&game_dir);
    }

    #[test]
    fn a_slug_that_is_not_a_plain_name_falls_back_to_the_id() {
        assert_eq!(dir_name(42, Some("hungario")), "hungario");
        assert_eq!(dir_name(42, Some("Hungario")), "hungario");
        assert_eq!(dir_name(42, Some("../etc")), "app-42");
        assert_eq!(dir_name(42, Some("a/b")), "app-42");
        assert_eq!(dir_name(42, Some("")), "app-42");
        assert_eq!(dir_name(42, None), "app-42");
        assert_eq!(dir_name(42, Some(&"x".repeat(65))), "app-42");
    }

    #[test]
    fn a_version_that_would_name_a_folder_elsewhere_is_refused() {
        assert!(version_dir("1.2.3").is_ok());
        assert!(version_dir("1.2.3-beta.1+abc").is_ok());
        // The update path DELETES sibling directories, so this one is a refusal
        // rather than a fallback: there is no safe name to fall back to.
        assert!(version_dir("../../etc").is_err());
        assert!(version_dir("a/b").is_err());
        assert!(version_dir("").is_err());
    }
}
