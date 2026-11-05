//! Materialising a subscription onto this machine, and taking it off again.
//!
//! This is the join between three things that are deliberately kept apart
//! everywhere else:
//!
//!   * a **subscription** — the account's statement that it wants this item;
//!   * an **app plugin** — the rule that says where this game's mods go;
//!   * the **jail and executor** — the only code that touches the disk.
//!
//! Nothing here resolves a path, opens a socket or writes a byte. It selects a
//! rule, builds the placeholder table, and hands both to
//! [`crate::plugins::steps::Executor`], which is the same executor a
//! user-installed plugin bundle runs through and enforces the same jail.
//!
//! THE INSTALL DIRECTORY QUESTION
//! ------------------------------
//! An install — what the app calls a **sandbox** — can point at its own directory on this
//! machine. When it does, that directory is the `gameDir` root for the run —
//! which is what makes two profiles for one game able to hold different mods
//! without either being aware of the other. When it does not, the app's
//! configured `game_dirs` entry is used, which is the "main install" every user
//! starts with.
//!
//! WHY UNINSTALL USES THE RECORDED FILE LIST
//! -----------------------------------------
//! A plugin's `uninstall` steps are the plugin author's idea of what to remove,
//! and they are run first. But the authoritative record of what an install
//! ACTUALLY wrote is the executor's own journal, stored on the row — so
//! anything left behind is removed from that list afterwards, bounded to paths
//! inside the same jail. A rule that was edited between install and
//! uninstall would otherwise strand files forever.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::audit;
use crate::error::{AppError, AppResult};
use crate::logging::Audit;
use crate::plugins::apps::{AppPluginFile, AppPluginKind, AppPlugins};
use crate::plugins::jail::Jail;
use crate::plugins::steps::{Executor, RunContext, RunReport};
use crate::plugins::{jail_for, JailRoots};
use crate::settings::AppSettings;

use super::db::{LibraryDb, LibraryEntry};

/// What one install or uninstall did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub id: String,
    pub ok: bool,
    /// The rule that ran, e.g. `minecraft/manage_mod.json`.
    pub rule: Option<String>,
    pub report: Option<RunReport>,
    pub error: Option<String>,
}

impl InstallOutcome {
    fn failed(id: &str, error: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            ok: false,
            rule: None,
            report: None,
            error: Some(error.into()),
        }
    }
}

/// Everything a run needs that is not the subscription itself.
pub struct InstallCtx<'a> {
    pub plugins: &'a AppPlugins,
    pub roots: &'a JailRoots,
    pub settings: &'a AppSettings,
    pub http: &'a reqwest::Client,
    pub audit: &'a Audit,
    /// The download queue, so an install's fetches are visible and pausable.
    pub downloads: Option<&'a crate::download::DownloadManager>,
    /// Which sandbox (cloud `install`) to materialise into. `None` = the app's main
    /// game directory.
    pub install_id: Option<i64>,
    /// That install's own directory on this machine, when it has one.
    pub install_dir: Option<PathBuf>,
    /// The install's declared loader, which a rule may match on.
    pub loader: Option<String>,
    /// The install's declared game version, offered as a placeholder.
    pub game_version: Option<String>,
    /// The install's name, offered as a placeholder.
    pub install_name: Option<String>,
}

/// The placeholder table a rule's steps interpolate.
///
/// A plain key→value map, filled by [`RunContext::fill`] with a literal
/// `{key}` → value replacement. Deliberately not a template LANGUAGE: an
/// expression evaluator here would be a scripting engine, which is the exact
/// thing the plugin model exists to avoid.
fn context_for(entry: &LibraryEntry, ctx: &InstallCtx<'_>) -> RunContext {
    let mut map: HashMap<String, String> = HashMap::new();

    map.insert("id".into(), entry.id.clone());
    map.insert("kind".into(), entry.kind.clone());
    map.insert("itemId".into(), entry.item_id.to_string());
    map.insert("itemName".into(), entry.name.clone());
    map.insert("webUrl".into(), entry.web_url.clone());

    if let Some(url) = &entry.file_url {
        map.insert("fileUrl".into(), url.clone());
    }

    /*
     * `fileName` is what a step writes into `mods/{fileName}`, so it has to be
     * a SAFE single component. The name comes from an uploader, so it is
     * sanitised here rather than trusted — the jail would refuse a traversal
     * anyway, but refusing it at run time means "install failed" rather than
     * "install produced a sensible name".
     */
    map.insert("fileName".into(), safe_file_name(entry));

    if let Some(sha) = entry.file_sha256.as_deref().filter(|s| !s.is_empty()) {
        map.insert("fileSha256".into(), sha.to_string());
    }

    if let Some(version) = &entry.latest_version {
        map.insert("version".into(), version.clone());
    }

    if let Some(release) = entry.latest_release_id {
        map.insert("releaseId".into(), release.to_string());
    }

    if let Some(slug) = &entry.app_slug {
        map.insert("appSlug".into(), slug.clone());
    }

    if let Some(name) = &ctx.install_name {
        map.insert("installName".into(), name.clone());
    }

    if let Some(version) = &ctx.game_version {
        map.insert("gameVersion".into(), version.clone());
    }

    if let Some(loader) = &ctx.loader {
        map.insert("loader".into(), loader.clone());
    }

    RunContext(map)
}

/// A file name that is safe as a single path component.
///
/// Falls back to `<kind>-<itemId>.bin` when there is nothing usable, because an
/// empty `{fileName}` produces a step that writes to a directory — which fails
/// with an error nobody can read.
pub(crate) fn safe_file_name(entry: &LibraryEntry) -> String {
    let raw = entry
        .file_name
        .as_deref()
        .or_else(|| {
            entry
                .file_url
                .as_deref()
                .and_then(|u| u.rsplit('/').next())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_default();

    // Take the last component and strip everything that is not a plain name
    // character. `..` collapses to nothing rather than to a dot pair.
    let base = raw.rsplit(['/', '\\']).next().unwrap_or_default();

    let mut out = String::with_capacity(base.len());
    let mut last_dot = false;

    for ch in base.chars() {
        let keep =
            ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '+' | '(' | ')' | ' ');

        if !keep {
            last_dot = false;
            out.push('-');
            continue;
        }

        // No `..` anywhere, in any encoding this can produce.
        if ch == '.' && last_dot {
            continue;
        }

        last_dot = ch == '.';
        out.push(ch);
    }

    let trimmed = out.trim_matches(|c: char| c == '.' || c == ' ').to_string();

    if trimmed.is_empty() || trimmed.len() > 180 {
        return format!("{}-{}.bin", entry.kind, entry.item_id);
    }

    trimmed
}

/// Which rule handles this item, if any.
pub fn rule_for<'a>(
    entry: &LibraryEntry,
    plugins: &'a AppPlugins,
    loader: Option<&str>,
) -> Option<&'a AppPluginFile> {
    let slug = entry.app_slug.as_deref()?;

    let kind = match entry.kind.as_str() {
        "mod" => AppPluginKind::ManageMod,
        "asset" => AppPluginKind::ManageAsset,
        /*
         * `ManageCollection` exists in the vocabulary but is never reached from
         * here: a collection has no app slug (collections are not app-scoped in
         * the schema), so the `?` above has already returned. The variant is
         * kept because a game MAY ship a rule that treats a whole collection as
         * one unit — a modpack manifest, say — and that is the hook for it.
         */
        "collection" => AppPluginKind::ManageCollection,
        _ => return None,
    };

    plugins.choose(slug, kind, &safe_file_name(entry), loader)
}

/// Build the jail for one run.
///
/// The install's own directory, when it has one, is injected by OVERRIDING the
/// app's configured `game_dirs` entry for this app id. That is deliberately the
/// same mechanism rather than a second one: `jail_for` already refuses a
/// directory that does not exist, and routing profiles through the same code
/// path means a profile cannot end up with laxer checks than the main install.
fn jail_for_run(
    rule: &AppPluginFile,
    entry: &LibraryEntry,
    ctx: &InstallCtx<'_>,
) -> AppResult<Jail> {
    let manifest = rule.as_manifest();

    let mut settings = ctx.settings.clone();

    if let (Some(app_id), Some(dir)) = (entry.app_id, ctx.install_dir.as_ref()) {
        settings
            .game_dirs
            .insert(app_id.to_string(), dir.display().to_string());
    }

    jail_for(&manifest, ctx.roots, &settings, entry.app_id)
}

/// Install (or update) one subscription.
///
/// Every refusal is a RETURNED outcome rather than an error, because the caller
/// is a loop over a library: one item whose game directory is unconfigured must
/// not stop the other forty from installing.
pub async fn install_one(
    db: &LibraryDb,
    entry: &LibraryEntry,
    ctx: &InstallCtx<'_>,
) -> InstallOutcome {
    if !entry.installable {
        return InstallOutcome::failed(
            &entry.id,
            "This item can no longer be installed by the app.",
        );
    }

    if entry.paused {
        return InstallOutcome::failed(&entry.id, "This subscription is paused.");
    }

    /*
     * A collection is a container. Subscribing to one created a real
     * subscription per member, and those are what get installed — see
     * `LibraryEntry::is_container`.
     */
    if entry.is_container() {
        return InstallOutcome::failed(
            &entry.id,
            "A collection is installed through its items, not on its own.",
        );
    }

    if entry.file_url.is_none() {
        return InstallOutcome::failed(&entry.id, "This item has no release file yet.");
    }

    let Some(rule) = rule_for(entry, ctx.plugins, ctx.loader.as_deref()) else {
        return InstallOutcome::failed(
            &entry.id,
            "No install rule for this game — the app cannot place its files.",
        );
    };

    let Some(manage) = &rule.manage else {
        return InstallOutcome::failed(&entry.id, "That rule declares no install steps.");
    };

    let _ = db.set_state(&entry.id, "installing", None);

    let jail = match jail_for_run(rule, entry, ctx) {
        Ok(jail) => jail,
        Err(err) => {
            let _ = db.set_state(&entry.id, "failed", Some(&err.to_string()));

            return InstallOutcome::failed(&entry.id, err.to_string());
        }
    };

    /*
     * An UPDATE takes the old version out of the way first, so `mods/` never
     * holds `cool-1.3.jar` and `cool-1.4.jar` together. It is MOVED aside, not
     * deleted: if the download, the checksum or any later step fails, the old
     * files are put back and the user still has a working mod. The uninstall
     * after that is best-effort - a file already deleted by hand is not a
     * reason to refuse the update.
     */
    let mut backup = UpdateBackup::new(db);

    if entry.installed_release_id.is_some() {
        if let Err(err) = backup.stash(entry, &jail) {
            let message = match backup.restore(&jail, &[]) {
                Ok(()) => {
                    let message = format!(
                        "The update did not start; the installed version was kept ({err})."
                    );

                    let _ = db.set_state(&entry.id, "installed", Some(&message));

                    message
                }
                Err(lost) => {
                    let message = format!(
                        "The update did not start ({err}) and the old files could not be put back: {lost}"
                    );

                    let _ = db.set_state(&entry.id, "failed", Some(&message));

                    message
                }
            };

            return InstallOutcome::failed(&entry.id, message);
        }

        let _ = run_uninstall(entry, ctx, rule, &jail).await;
    }

    let manifest = rule.as_manifest();

    let executor = Executor {
        manifest: &manifest,
        jail: &jail,
        http: ctx.http,
        audit: ctx.audit,
        downloads: ctx.downloads,
    };

    let run_ctx = context_for(entry, ctx);

    audit!(
        ctx.audit,
        Info,
        Install,
        "library.install.start",
        format!("{} ({})", entry.name, rule.source),
        plugin = rule.source
    );

    let report = executor.run(&manage.install, &run_ctx).await;

    if report.ok {
        let _ = db.mark_installed(
            &entry.id,
            entry.latest_release_id,
            entry.latest_version.as_deref(),
            ctx.install_id,
            &report.applied,
        );

        backup.discard();

        audit!(
            ctx.audit,
            Info,
            Install,
            "library.install.ok",
            format!("{} → {} file(s)", entry.name, report.applied.len()),
            plugin = rule.source
        );
    } else {
        let message = report
            .error
            .clone()
            .unwrap_or_else(|| "The install did not finish.".into());

        if backup.is_empty() {
            let _ = db.set_state(&entry.id, "failed", Some(&message));
        } else {
            match backup.restore(&jail, &report.applied) {
                Ok(()) => {
                    let kept = format!("Update failed, the installed version was kept: {message}");

                    let _ = db.set_state(&entry.id, "installed", Some(&kept));
                }
                Err(lost) => {
                    let broken = format!(
                        "Update failed ({message}) and the previous version could not be put back: {lost}"
                    );

                    let _ = db.set_state(&entry.id, "failed", Some(&broken));
                }
            }
        }

        audit!(
            ctx.audit,
            Error,
            Install,
            "library.install.fail",
            format!("{}: {message}", entry.name),
            plugin = rule.source
        );
    }

    InstallOutcome {
        id: entry.id.clone(),
        ok: report.ok,
        rule: Some(rule.source.clone()),
        error: report.error.clone(),
        report: Some(report),
    }
}

const SUFFIX: &str = ".tmc-old";

/// The meta key listing every stash that has not been resolved yet.
const PENDING_KEY: &str = "install.pending_backups";

static PENDING_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One path moved aside by an update and not yet restored or deleted.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct Pending {
    original: String,
    waiting: String,
    /// The new version is installed, so `waiting` is only garbage.
    committed: bool,
}

fn waiting_for(path: &Path) -> PathBuf {
    let mut waiting = path.as_os_str().to_os_string();
    waiting.push(SUFFIX);
    PathBuf::from(waiting)
}

fn pending_edit(db: &LibraryDb, edit: impl FnOnce(&mut Vec<Pending>)) -> AppResult<()> {
    let _guard = PENDING_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut list: Vec<Pending> = db
        .meta_get(PENDING_KEY)?
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();

    edit(&mut list);

    db.meta_set(PENDING_KEY, &serde_json::to_string(&list)?)
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(_) => Ok(()),
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
    }
}

/// Settle stashes an earlier run left behind (a crash, or a delete the OS refused).
///
/// Call once at startup, before anything installs. A path whose real file is
/// gone gets its old copy back; one whose update finished has its copy deleted;
/// an unfinished update that left a new file in place keeps the old copy
/// beside it for the user rather than guessing. Returns how many were restored.
pub fn recover_interrupted(db: &LibraryDb) -> usize {
    let mut restored = 0;

    let result = pending_edit(db, |list| {
        let mut keep: Vec<Pending> = Vec::new();

        // Newest first, so a directory comes back before the files stashed inside it.
        for item in list.drain(..).rev() {
            let original = PathBuf::from(&item.original);
            let waiting = PathBuf::from(&item.waiting);

            // The row is user-writable: only ever act on `<path>.tmc-old` next to `<path>`.
            if waiting != waiting_for(&original) || !original.is_absolute() {
                continue;
            }

            if std::fs::symlink_metadata(&waiting).is_err() {
                continue;
            }

            if std::fs::symlink_metadata(&original).is_err() {
                match std::fs::rename(&waiting, &original) {
                    Ok(()) => restored += 1,
                    Err(_) => keep.push(item),
                }
            } else if item.committed {
                if remove_any(&waiting).is_err() {
                    keep.push(item);
                }
            } else {
                tracing::warn!(
                    "an interrupted update left {} beside a newer file; kept for the user",
                    waiting.display()
                );
            }
        }

        keep.reverse();
        *list = keep;
    });

    if let Err(err) = result {
        tracing::warn!("could not settle interrupted updates: {}", err.detail());
    }

    restored
}

/// The files of the installed version, moved aside while its replacement installs.
#[derive(Default)]
struct UpdateBackup<'a> {
    db: Option<&'a LibraryDb>,
    /// (original path, where it waits)
    moved: Vec<(PathBuf, PathBuf)>,
}

impl<'a> UpdateBackup<'a> {
    fn new(db: &'a LibraryDb) -> Self {
        Self {
            db: Some(db),
            moved: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.moved.is_empty()
    }

    /// Is everything under `dir` one of the journaled paths (so nothing foreign moves with it)?
    fn owned(dir: &Path, journal: &HashSet<PathBuf>, memo: &mut HashMap<PathBuf, bool>) -> bool {
        if let Some(&known) = memo.get(dir) {
            return known;
        }

        let mut ok = std::fs::read_dir(dir).is_ok();

        if ok {
            for child in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = child.path();
                let is_dir = child.file_type().map(|t| t.is_dir()).unwrap_or(false);

                let fine = if is_dir {
                    Self::owned(&path, journal, memo)
                } else {
                    journal.contains(&path)
                };

                if !fine {
                    ok = false;
                    break;
                }
            }
        }

        memo.insert(dir.to_path_buf(), ok);

        ok
    }

    /// Move the installed version's files, and any folder holding only them, beside themselves as `<name>.tmc-old`.
    ///
    /// Same parent, so the rename is atomic and never crosses a volume. A folder goes whole
    /// only when every file in it is journaled and it is not a shared grant folder: a rule
    /// that removes a mod's folder would otherwise take the stashed copies with it. Paths
    /// are re-checked against the jail like the uninstall sweep does: the journal sits in
    /// a user-writable database. Each move is recorded in the library first, so a crash
    /// between stash and restore is repaired by [`recover_interrupted`].
    fn stash(&mut self, entry: &LibraryEntry, jail: &Jail) -> std::io::Result<()> {
        let journal: HashSet<PathBuf> = entry
            .installed_files
            .iter()
            .filter(|r| !r.starts_with("removed "))
            .map(PathBuf::from)
            .filter(|p| jail.contains(p))
            .collect();

        let mut memo: HashMap<PathBuf, bool> = HashMap::new();
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();

        for path in &journal {
            let meta = std::fs::symlink_metadata(path);
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let is_file = meta.as_ref().map(|m| m.is_file()).unwrap_or(false);

            if !is_dir && !is_file {
                continue;
            }

            let mut top: Option<PathBuf> = None;
            let mut cursor = if is_dir {
                Some(path.clone())
            } else {
                path.parent().map(Path::to_path_buf)
            };

            while let Some(dir) = cursor {
                if jail.is_boundary(&dir)
                    || !jail.contains(&dir)
                    || !Self::owned(&dir, &journal, &mut memo)
                {
                    break;
                }

                cursor = dir.parent().map(Path::to_path_buf);
                top = Some(dir);
            }

            match top {
                Some(dir) => dirs.push(dir),
                None if is_file => files.push(path.clone()),
                None => {}
            }
        }

        dirs.sort_by_key(|d| d.components().count());
        dirs.dedup();
        files.sort();

        let mut taken: Vec<PathBuf> = Vec::new();

        for path in dirs.into_iter().chain(files) {
            if taken.iter().any(|d| path.starts_with(d)) {
                continue;
            }

            self.move_aside(&path)?;
            taken.push(path);
        }

        Ok(())
    }

    fn move_aside(&mut self, path: &Path) -> std::io::Result<()> {
        let waiting = waiting_for(path);

        // The real file exists, so a copy already waiting is a leftover from an earlier
        // update that could not delete it. Clear it; if that fails, refuse to start.
        if std::fs::symlink_metadata(&waiting).is_ok() {
            remove_any(&waiting).map_err(|e| {
                std::io::Error::new(
                    e.kind(),
                    format!("an old backup {} cannot be removed: {e}", waiting.display()),
                )
            })?;
        }

        if let Some(db) = self.db {
            let item = Pending {
                original: path.display().to_string(),
                waiting: waiting.display().to_string(),
                committed: false,
            };

            pending_edit(db, |list| list.push(item))
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }

        if let Err(err) = std::fs::rename(path, &waiting) {
            self.forget(&waiting);

            return Err(err);
        }

        self.moved.push((path.to_path_buf(), waiting));

        Ok(())
    }

    fn forget(&self, waiting: &Path) {
        if let Some(db) = self.db {
            let key = waiting.display().to_string();
            let _ = pending_edit(db, |list| list.retain(|p| p.waiting != key));
        }
    }

    /// The update worked: the old files are no longer wanted.
    ///
    /// A copy the OS will not delete (a locked file on Windows) stays in the library's
    /// pending list, marked finished, and [`recover_interrupted`] deletes it later.
    fn discard(&mut self) {
        if let Some(db) = self.db {
            let waiting: HashSet<String> = self
                .moved
                .iter()
                .map(|(_, w)| w.display().to_string())
                .collect();

            let _ = pending_edit(db, |list| {
                for item in list.iter_mut().filter(|p| waiting.contains(&p.waiting)) {
                    item.committed = true;
                }
            });
        }

        for (_, waiting) in std::mem::take(&mut self.moved) {
            if remove_any(&waiting).is_ok() {
                self.forget(&waiting);
            } else {
                tracing::warn!(
                    "could not delete {}; it will be retried at next start",
                    waiting.display()
                );
            }
        }
    }

    /// The update failed: remove what it wrote, put the old files back.
    ///
    /// Whatever cannot be put back stays waiting, listed in the library, and the error
    /// names it; the caller must not report the old version as intact.
    fn restore(&mut self, jail: &Jail, written: &[String]) -> Result<(), String> {
        let mut paths: Vec<PathBuf> = written
            .iter()
            .filter(|r| !r.starts_with("removed "))
            .map(PathBuf::from)
            .filter(|p| jail.contains(p))
            .collect();

        // Deepest first, so a folder is empty by the time it is tried.
        paths.sort_by_key(|p| std::cmp::Reverse(p.components().count()));

        for path in paths {
            if path.is_dir() {
                let _ = std::fs::remove_dir(&path);
            } else if path.is_file() {
                let _ = std::fs::remove_file(&path);
            }
        }

        let mut errors: Vec<String> = Vec::new();

        for (original, waiting) in std::mem::take(&mut self.moved).into_iter().rev() {
            match std::fs::rename(&waiting, &original) {
                Ok(()) => self.forget(&waiting),
                Err(err) => errors.push(format!("{} ({err})", original.display())),
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// Run a rule's uninstall steps, then sweep anything its journal recorded.
async fn run_uninstall(
    entry: &LibraryEntry,
    ctx: &InstallCtx<'_>,
    rule: &AppPluginFile,
    jail: &Jail,
) -> RunReport {
    let manifest = rule.as_manifest();

    let executor = Executor {
        manifest: &manifest,
        jail,
        http: ctx.http,
        audit: ctx.audit,
        downloads: ctx.downloads,
    };

    let steps = rule
        .manage
        .as_ref()
        .map(|m| m.uninstall.as_slice())
        .unwrap_or_default();

    let report = executor.run(steps, &context_for(entry, ctx)).await;

    /*
     * The journal sweep.
     *
     * The rule's own uninstall steps are the author's idea of what to remove;
     * `installed_files` is what the executor ACTUALLY wrote. They usually
     * agree, and when they do this removes nothing. When they do not — a rule
     * edited between install and uninstall, a step whose target moved — this is
     * what stops a file being stranded forever.
     *
     * Bounded to paths INSIDE the jail's roots. The list is our own record,
     * but it is on disk in a user-writable database, so it is re-checked rather
     * than trusted: an edited row must not become a delete-anything primitive.
     */
    for recorded in &entry.installed_files {
        // The executor writes `removed <path>` for a delete; there is nothing
        // left to sweep for one.
        if recorded.starts_with("removed ") {
            continue;
        }

        let path = PathBuf::from(recorded);

        if !jail.contains(&path) {
            continue;
        }

        if path.is_dir() {
            // Only if empty. A directory the rule created inside the game
            // folder may hold files the user put there.
            let _ = std::fs::remove_dir(&path);
        } else if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }

    report
}

/// Uninstall one subscription from this machine.
///
/// Leaves the subscription row in place — the account may still be subscribed
/// and simply have paused it. `forget` deletes the row too, which is what a
/// full sync does for something unsubscribed elsewhere.
pub async fn uninstall_one(
    db: &LibraryDb,
    entry: &LibraryEntry,
    ctx: &InstallCtx<'_>,
    forget: bool,
) -> InstallOutcome {
    let Some(rule) = rule_for(entry, ctx.plugins, ctx.loader.as_deref()) else {
        /*
         * No rule any more — the game lost support, or the file was moved into
         * `disabled/`. The journal sweep is still possible and is still the
         * right thing to do, so a bare jail is built from the recorded
         * paths' own roots rather than refusing.
         *
         * Without a rule there is no manifest and therefore no jail, so the
         * only honest answer is to leave the files and say so. Deleting paths
         * off a database row with no jail to check them against is exactly the
         * primitive this whole module exists to not have.
         */
        let _ = db.set_state(
            &entry.id,
            "failed",
            Some("No install rule for this game — its files were left in place."),
        );

        return InstallOutcome::failed(
            &entry.id,
            "No install rule for this game — its files were left in place.",
        );
    };

    let _ = db.set_state(&entry.id, "removing", None);

    let jail = match jail_for_run(rule, entry, ctx) {
        Ok(jail) => jail,
        Err(err) => {
            let _ = db.set_state(&entry.id, "failed", Some(&err.to_string()));

            return InstallOutcome::failed(&entry.id, err.to_string());
        }
    };

    audit!(
        ctx.audit,
        Info,
        Install,
        "library.uninstall.start",
        format!("{} ({})", entry.name, rule.source),
        plugin = rule.source
    );

    let report = run_uninstall(entry, ctx, rule, &jail).await;

    if forget {
        let _ = db.delete(&entry.id);
    } else {
        let _ = db.mark_uninstalled(&entry.id);
    }

    audit!(
        ctx.audit,
        Info,
        Install,
        "library.uninstall.ok",
        entry.name.clone(),
        plugin = rule.source
    );

    InstallOutcome {
        id: entry.id.clone(),
        ok: true,
        rule: Some(rule.source.clone()),
        error: report.error.clone(),
        report: Some(report),
    }
}

/// A refusal the UI can render before anything runs.
///
/// Used by the "install now" button so a missing game directory is a sentence
/// rather than a failed run halfway through a download.
pub fn preflight(entry: &LibraryEntry, ctx: &InstallCtx<'_>) -> AppResult<()> {
    if !entry.installable {
        return Err(AppError::invalid(
            "This item can no longer be installed by the app.",
        ));
    }

    if entry.is_container() {
        return Err(AppError::invalid(
            "A collection is installed through its items, not on its own.",
        ));
    }

    if entry.file_url.is_none() {
        return Err(AppError::invalid("This item has no release file yet."));
    }

    let Some(rule) = rule_for(entry, ctx.plugins, ctx.loader.as_deref()) else {
        return Err(AppError::invalid(
            "No install rule for this game — the app cannot place its files.",
        ));
    };

    jail_for_run(rule, entry, ctx)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: Option<&str>, url: Option<&str>) -> LibraryEntry {
        LibraryEntry {
            id: "s1".into(),
            kind: "mod".into(),
            item_id: 42,
            name: "Cool Mod".into(),
            description: None,
            image: None,
            web_url: "https://example.com".into(),
            app_id: Some(1),
            app_name: None,
            app_slug: Some("minecraft".into()),
            auto_update: true,
            notify_updates: true,
            paused: false,
            via_collection_id: None,
            installable: true,
            latest_release_id: Some(1),
            latest_version: None,
            file_url: url.map(str::to_string),
            file_name: name.map(str::to_string),
            file_size: None,
            file_sha256: None,
            updated_at: String::new(),
            installed_release_id: None,
            installed_version: None,
            installed_install_id: None,
            installed_at: None,
            installed_files: vec![],
            state: "idle".into(),
            last_error: None,
        }
    }

    #[test]
    fn a_file_name_can_never_climb_out_of_a_directory() {
        for bad in [
            "../../etc/passwd",
            "..\\..\\windows\\system32\\evil.dll",
            "....//evil.jar",
            "/absolute.jar",
            "C:\\abs.jar",
        ] {
            let name = safe_file_name(&entry(Some(bad), None));

            assert!(!name.contains(".."), "{bad} → {name}");
            assert!(!name.contains('/'), "{bad} → {name}");
            assert!(!name.contains('\\'), "{bad} → {name}");
            assert!(!name.starts_with('.'), "{bad} → {name}");
        }
    }

    #[test]
    fn an_ordinary_name_survives_intact() {
        assert_eq!(
            safe_file_name(&entry(Some("Cool Mod (1.4).jar"), None)),
            "Cool Mod (1.4).jar"
        );
    }

    #[test]
    fn a_missing_name_falls_back_to_the_url_then_to_an_id() {
        assert_eq!(
            safe_file_name(&entry(None, Some("https://x/download/abc.jar"))),
            "abc.jar"
        );

        // A download route with no filename — which is exactly what
        // `/download/<uuid>` is — still produces a usable component.
        assert_eq!(
            safe_file_name(&entry(None, Some("https://x/download/"))),
            "mod-42.bin"
        );

        assert_eq!(safe_file_name(&entry(None, None)), "mod-42.bin");
    }

    #[test]
    fn an_absurd_name_falls_back_rather_than_being_truncated() {
        let long = "a".repeat(300);

        assert_eq!(safe_file_name(&entry(Some(&long), None)), "mod-42.bin");
    }

    const RULE: &str = r#"{
        "manifestVersion": 1,
        "label": "test jar",
        "match": { "extensions": ["jar"] },
        "permissions": {
            "fs": [
                { "root": "pluginData", "path": "", "write": true },
                { "root": "gameDir", "path": "mods", "write": true }
            ],
            "net": ["localhost"]
        },
        "manage": {
            "install": [
                { "action": "download", "url": "{fileUrl}",
                  "to": { "root": "pluginData", "path": "staging/{fileName}" } },
                { "action": "move",
                  "from": { "root": "pluginData", "path": "staging/{fileName}" },
                  "to": { "root": "gameDir", "path": "mods/{fileName}" } }
            ],
            "uninstall": [
                { "action": "remove", "path": { "root": "gameDir", "path": "mods/{fileName}" } }
            ]
        }
    }"#;

    #[tokio::test]
    async fn a_failed_update_keeps_the_installed_version() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let rules = tmp.path().join("rules");
        let game = tmp.path().join("game");
        let jar = game.join("mods").join("cool.jar");

        std::fs::create_dir_all(rules.join("app/minecraft")).unwrap();
        std::fs::write(rules.join("app/minecraft/manage_mod.json"), RULE).unwrap();
        std::fs::create_dir_all(jar.parent().unwrap()).unwrap();
        std::fs::write(&jar, b"old version").unwrap();

        let plugins = AppPlugins::load(&rules);
        let roots = JailRoots {
            data: tmp.path().join("data"),
            cache: tmp.path().join("cache"),
        };
        let mut settings = AppSettings::default();
        settings
            .game_dirs
            .insert("1".into(), game.display().to_string());

        let http = crate::http::client();
        let audit = Audit::new(tmp.path().join("audit.jsonl"));
        let ctx = InstallCtx {
            plugins: &plugins,
            roots: &roots,
            settings: &settings,
            http: &http,
            audit: &audit,
            downloads: None,
            install_id: None,
            install_dir: None,
            loader: None,
            game_version: None,
            install_name: None,
        };

        // Nothing listens on port 1, so the download of the new version fails.
        let mut e = entry(Some("cool.jar"), Some("https://localhost:1/cool.jar"));
        e.installed_release_id = Some(0);
        e.installed_version = Some("1.0".into());
        e.installed_files = vec![jar.display().to_string()];

        let db = LibraryDb::open_memory().expect("db");
        db.upsert_remote(&e).expect("row");
        db.mark_installed(&e.id, Some(0), Some("1.0"), None, &e.installed_files)
            .expect("installed");

        let outcome = install_one(&db, &e, &ctx).await;

        assert!(!outcome.ok, "the download cannot succeed");
        assert_eq!(std::fs::read(&jar).unwrap(), b"old version");
        assert!(!jar.with_extension("jar.tmc-old").exists());

        let row = db.get(&e.id).unwrap().unwrap();
        assert_eq!(row.state, "installed");
        assert_eq!(row.installed_version.as_deref(), Some("1.0"));
        assert_eq!(row.installed_files, e.installed_files);
    }

    fn test_jail(root: &Path) -> Jail {
        let mut available = std::collections::HashMap::new();
        available.insert("gameDir", root.to_path_buf());
        let m = crate::plugins::manifest::Manifest {
            manifest_version: 1,
            id: "app.test".into(),
            name: "t".into(),
            version: "1".into(),
            author: "t".into(),
            description: None,
            homepage: None,
            apps: vec![],
            permissions: crate::plugins::manifest::Permissions {
                fs: vec![crate::plugins::manifest::FsGrant {
                    root: crate::plugins::manifest::FsRoot::GameDir,
                    path: String::new(),
                    write: true,
                }],
                ..Default::default()
            },
            installer: None,
            server_query: None,
            theme: None,
            manager: None,
        };

        Jail::build(&m, &available).expect("jail")
    }

    #[test]
    fn a_backup_is_restored_over_what_the_failed_run_wrote() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("a.jar");
        std::fs::write(&file, b"old").unwrap();

        let jail = test_jail(tmp.path());

        let mut e = entry(Some("a.jar"), None);
        e.installed_files = vec![file.display().to_string()];

        let mut backup = UpdateBackup::default();
        backup.stash(&e, &jail).expect("stash");
        assert!(!file.exists());

        std::fs::write(&file, b"half written new").unwrap();
        backup
            .restore(&jail, &[file.display().to_string()])
            .expect("restore");

        assert_eq!(std::fs::read(&file).unwrap(), b"old");
    }

    #[test]
    fn a_folder_holding_only_the_mod_is_stashed_whole_and_restored() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mods = tmp.path().join("mods");
        let own = mods.join("Cool");
        std::fs::create_dir_all(own.join("sub")).unwrap();
        std::fs::write(own.join("a.txt"), b"a").unwrap();
        std::fs::write(own.join("sub/b.txt"), b"b").unwrap();
        std::fs::write(mods.join("other.jar"), b"theirs").unwrap();

        let jail = test_jail(tmp.path());
        let mut e = entry(Some("a.jar"), None);
        e.installed_files = vec![
            own.join("a.txt").display().to_string(),
            own.join("sub/b.txt").display().to_string(),
        ];

        let db = LibraryDb::open_memory().expect("db");
        let mut backup = UpdateBackup::new(&db);
        backup.stash(&e, &jail).expect("stash");

        // The folder moved as one, the neighbour stayed.
        assert!(!own.exists());
        assert!(waiting_for(&own).join("sub/b.txt").exists());
        assert!(mods.join("other.jar").exists());
        assert_eq!(backup.moved.len(), 1);

        backup.restore(&jail, &[]).expect("restore");

        assert_eq!(std::fs::read(own.join("sub/b.txt")).unwrap(), b"b");
        assert!(!waiting_for(&own).exists());
        assert!(pending_is_empty(&db));
    }

    fn pending_is_empty(db: &LibraryDb) -> bool {
        db.meta_get(PENDING_KEY)
            .unwrap()
            .map(|t| t == "[]")
            .unwrap_or(true)
    }

    #[test]
    fn a_restore_that_fails_says_so_and_keeps_the_copy_listed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("a.jar");
        std::fs::write(&file, b"old").unwrap();

        let jail = test_jail(tmp.path());
        let mut e = entry(Some("a.jar"), None);
        e.installed_files = vec![file.display().to_string()];

        let db = LibraryDb::open_memory().expect("db");
        let mut backup = UpdateBackup::new(&db);
        backup.stash(&e, &jail).expect("stash");

        // Something unremovable now sits where the old file belongs.
        std::fs::create_dir_all(file.join("blocker")).unwrap();
        std::fs::write(file.join("blocker/x"), b"x").unwrap();

        let err = backup
            .restore(&jail, &[])
            .expect_err("must not claim success");

        assert!(err.contains("a.jar"), "{err}");
        assert!(waiting_for(&file).exists());
        assert!(!pending_is_empty(&db));
    }

    #[test]
    fn a_crash_between_stash_and_restore_is_repaired_at_startup() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let lost = tmp.path().join("lost.jar");
        let done = tmp.path().join("done.jar");
        let half = tmp.path().join("half.jar");

        let jail = test_jail(tmp.path());
        let db = LibraryDb::open_memory().expect("db");

        for (file, bytes) in [
            (&lost, "old lost"),
            (&done, "old done"),
            (&half, "old half"),
        ] {
            std::fs::write(file, bytes).unwrap();

            let mut e = entry(Some("x.jar"), None);
            e.installed_files = vec![file.display().to_string()];

            let mut backup = UpdateBackup::new(&db);
            backup.stash(&e, &jail).expect("stash");
            // Dropped without restore or discard: the process died here.
        }

        // `done`: the new version landed and the update was committed; `half`: it did not commit.
        std::fs::write(&done, b"new").unwrap();
        std::fs::write(&half, b"partial").unwrap();
        pending_edit(&db, |list| {
            for item in list.iter_mut() {
                item.committed = item.original.ends_with("done.jar");
            }
        })
        .unwrap();

        assert_eq!(recover_interrupted(&db), 1);

        assert_eq!(std::fs::read(&lost).unwrap(), b"old lost");
        assert!(!waiting_for(&lost).exists());
        assert_eq!(std::fs::read(&done).unwrap(), b"new");
        assert!(!waiting_for(&done).exists());
        // Never guess over a newer file: the old copy is left for the user.
        assert_eq!(std::fs::read(&half).unwrap(), b"partial");
        assert!(waiting_for(&half).exists());
    }

    #[test]
    fn a_stale_copy_beside_an_installed_file_is_cleared_not_trusted() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("a.jar");
        std::fs::write(&file, b"current").unwrap();
        std::fs::write(waiting_for(&file), b"stale").unwrap();

        let jail = test_jail(tmp.path());
        let mut e = entry(Some("a.jar"), None);
        e.installed_files = vec![file.display().to_string()];

        let mut backup = UpdateBackup::default();
        backup.stash(&e, &jail).expect("stash");

        assert_eq!(std::fs::read(waiting_for(&file)).unwrap(), b"current");
    }
}
