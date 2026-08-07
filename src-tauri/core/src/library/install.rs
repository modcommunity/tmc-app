//! Materialising a subscription onto this machine, and taking it off again.
//!
//! This is the join between three things that are deliberately kept apart
//! everywhere else:
//!
//!   * a **subscription** — the account's statement that it wants this item;
//!   * an **app plugin** — the rule that says where this game's mods go;
//!   * the **sandbox and executor** — the only code that touches the disk.
//!
//! Nothing here resolves a path, opens a socket or writes a byte. It selects a
//! rule, builds the placeholder table, and hands both to
//! [`crate::plugins::steps::Executor`], which is the same executor a
//! user-installed plugin bundle runs through and enforces the same jail.
//!
//! THE INSTALL DIRECTORY QUESTION
//! ------------------------------
//! An install (a sandbox/profile) can point at its own directory on this
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
//! inside the same sandbox. A rule that was edited between install and
//! uninstall would otherwise strand files forever.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::audit;
use crate::error::{AppError, AppResult};
use crate::logging::Audit;
use crate::plugins::apps::{AppPluginFile, AppPluginKind, AppPlugins};
use crate::plugins::sandbox::Sandbox;
use crate::plugins::steps::{Executor, RunContext, RunReport};
use crate::plugins::{sandbox_for, SandboxRoots};
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
    pub roots: &'a SandboxRoots,
    pub settings: &'a AppSettings,
    pub http: &'a reqwest::Client,
    pub audit: &'a Audit,
    /// Which install (sandbox) to materialise into. `None` = the app's main
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
     * sanitised here rather than trusted — the sandbox would refuse a traversal
     * anyway, but refusing it at run time means "install failed" rather than
     * "install produced a sensible name".
     */
    map.insert("fileName".into(), safe_file_name(entry));

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
fn safe_file_name(entry: &LibraryEntry) -> String {
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

/// Build the sandbox for one run.
///
/// The install's own directory, when it has one, is injected by OVERRIDING the
/// app's configured `game_dirs` entry for this app id. That is deliberately the
/// same mechanism rather than a second one: `sandbox_for` already refuses a
/// directory that does not exist, and routing profiles through the same code
/// path means a profile cannot end up with laxer checks than the main install.
fn sandbox_for_run(
    rule: &AppPluginFile,
    entry: &LibraryEntry,
    ctx: &InstallCtx<'_>,
) -> AppResult<Sandbox> {
    let manifest = rule.as_manifest();

    let mut settings = ctx.settings.clone();

    if let (Some(app_id), Some(dir)) = (entry.app_id, ctx.install_dir.as_ref()) {
        settings
            .game_dirs
            .insert(app_id.to_string(), dir.display().to_string());
    }

    sandbox_for(&manifest, ctx.roots, &settings, entry.app_id)
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

    let sandbox = match sandbox_for_run(rule, entry, ctx) {
        Ok(sandbox) => sandbox,
        Err(err) => {
            let _ = db.set_state(&entry.id, "failed", Some(&err.to_string()));

            return InstallOutcome::failed(&entry.id, err.to_string());
        }
    };

    /*
     * An UPDATE removes the old version first.
     *
     * Not doing so is the classic mod-manager bug: `mods/` ends up holding
     * `cool-1.3.jar` and `cool-1.4.jar`, the game loads both, and the crash
     * that follows is blamed on the mod. The uninstall is best-effort — a file
     * a user already deleted by hand is not a reason to refuse the update.
     */
    if entry.installed_release_id.is_some() {
        let _ = run_uninstall(entry, ctx, rule, &sandbox).await;
    }

    let manifest = rule.as_manifest();

    let executor = Executor {
        manifest: &manifest,
        sandbox: &sandbox,
        http: ctx.http,
        audit: ctx.audit,
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

        let _ = db.set_state(&entry.id, "failed", Some(&message));

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

/// Run a rule's uninstall steps, then sweep anything its journal recorded.
async fn run_uninstall(
    entry: &LibraryEntry,
    ctx: &InstallCtx<'_>,
    rule: &AppPluginFile,
    sandbox: &Sandbox,
) -> RunReport {
    let manifest = rule.as_manifest();

    let executor = Executor {
        manifest: &manifest,
        sandbox,
        http: ctx.http,
        audit: ctx.audit,
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
     * Bounded to paths INSIDE the sandbox's roots. The list is our own record,
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

        if !sandbox.contains(&path) {
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
         * right thing to do, so a bare sandbox is built from the recorded
         * paths' own roots rather than refusing.
         *
         * Without a rule there is no manifest and therefore no sandbox, so the
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

    let sandbox = match sandbox_for_run(rule, entry, ctx) {
        Ok(sandbox) => sandbox,
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

    let report = run_uninstall(entry, ctx, rule, &sandbox).await;

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

    sandbox_for_run(rule, entry, ctx)?;

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
}
