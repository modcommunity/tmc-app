//! Staging a mod into a sandbox, and putting that sandbox in front of the game.
//!
//! The join between four things that are deliberately separate everywhere else:
//! a **subscription** (the account wants this item), a **sandbox** (this profile
//! wants it at this priority), an **install rule** (where this game's mods go)
//! and the **deployment engine** (how files reach the game folder).
//!
//! TWO STEPS, NOT ONE
//! ------------------
//! ```text
//!   stage   — run the game's install rule with the sandbox's own staging
//!             folder standing in for the game directory
//!   deploy  — mirror every staged file into the real game folder, by
//!             whichever mechanism the sandbox is set to
//! ```
//!
//! Splitting them is what makes the whole feature work, and it is worth being
//! explicit about why:
//!
//!   * **Every install rule written before sandboxes existed still works.** A
//!     rule that copies to `mods/{fileName}` writes to
//!     `<staging>/<sandbox>/<mod>/mods/foo.jar`, and deployment puts
//!     `mods/foo.jar` in the game folder. The rule never learns which strategy
//!     is in use, and it should not — "where does a Minecraft mod go" and "how
//!     do files reach the game folder" have different right answers.
//!   * **Switching sandboxes is a link operation, not a download.** Staging
//!     survives an undeploy, so the mods for the profile you just left are
//!     still on disk when you come back.
//!   * **Nothing partially-downloaded is ever in the game folder.** A failed
//!     stage leaves a mess inside the app's own staging directory, which
//!     nothing else reads.
//!
//! The cost is disk: a direct-strategy sandbox holds each file twice, once in
//! staging and once in the game. That is the price of being able to undeploy
//! exactly, and it is why the linking strategies are the default.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::audit;
use crate::deploy::{self, DeployReport, DeployRequest, LedgerEntry, PurgeReport, Strategy};
use crate::error::{AppError, AppResult};
use crate::launch::{LaunchContext, LaunchOptions, LaunchPlan, VfsHandoff};
use crate::logging::Audit;
use crate::plugins::apps::{AppPluginKind, AppPlugins};
use crate::plugins::steps::{Executor, RunContext};
use crate::plugins::{jail_for, jail_for_scoped, JailRoots};
use crate::settings::AppSettings;

use super::db::{LibraryDb, LibraryEntry};
use super::sandbox::{Sandbox, SandboxMod};

/// Everything a sandbox operation needs that is not the sandbox.
pub struct SandboxCtx<'a> {
    pub plugins: &'a AppPlugins,
    pub roots: &'a JailRoots,
    pub settings: &'a AppSettings,
    pub http: &'a reqwest::Client,
    pub audit: &'a Audit,
    /// The download queue, so staging a mod is visible and pausable.
    pub downloads: Option<&'a crate::download::DownloadManager>,
    /// Where every sandbox's staging folders live: `<staging>/<id>/<mod key>`.
    pub staging_root: &'a Path,
    /// Where imported mods live: `<local>/<local mod id>`.
    ///
    /// A DEVICE-wide directory, not a per-sandbox one, and that asymmetry is
    /// the point. A staged subscription is this sandbox's copy at this
    /// sandbox's pinned release; an imported mod is one set of files the user
    /// put on the machine once, and three sandboxes using it should be three
    /// references rather than three copies. Deployment only ever READS a mod's
    /// root, which is what makes sharing one safe — the same property that lets
    /// two sandboxes share a staging folder.
    pub local_root: &'a Path,
    /// Where displaced game files go: `<backups>/<id>/<relative path>`.
    pub backup_root: &'a Path,
}

/// What staging one mod did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageOutcome {
    pub mod_key: String,
    pub ok: bool,
    pub rule: Option<String>,
    /// Files written into the staging folder.
    pub files: usize,
    pub error: Option<String>,
}

impl StageOutcome {
    fn failed(mod_key: &str, error: impl Into<String>) -> Self {
        Self {
            mod_key: mod_key.to_string(),
            ok: false,
            rule: None,
            files: 0,
            error: Some(error.into()),
        }
    }
}

/// Where a sandbox deploys to on this machine.
///
/// Its own folder when it has one — which is how CurseForge-style isolated
/// instances are expressed — and otherwise the app's configured directory for
/// the game, which is the "main install" every user starts with.
///
/// A missing directory is an ERROR rather than something to create: creating it
/// means a typo in the settings produces an empty folder that the deploy then
/// "succeeds" into, leaving the user with a game that has no mods and an app
/// that says it worked.
pub fn target_dir(sandbox: &Sandbox, settings: &AppSettings) -> AppResult<PathBuf> {
    let raw = sandbox
        .game_dir
        .clone()
        .or_else(|| settings.game_dirs.get(&sandbox.app_id.to_string()).cloned())
        .ok_or_else(|| {
            AppError::invalid(
                "This sandbox has no game folder yet. Set one in Settings → Games, or give the \
                 sandbox its own folder.",
            )
        })?;

    let path = PathBuf::from(raw);

    if !path.is_dir() {
        return Err(AppError::invalid(format!(
            "The folder this sandbox deploys into does not exist: {}",
            path.display()
        )));
    }

    Ok(path)
}

/// Which rule stages this item for this sandbox.
fn rule_kind(kind: &str) -> Option<AppPluginKind> {
    match kind {
        "mod" => Some(AppPluginKind::ManageMod),
        "asset" => Some(AppPluginKind::ManageAsset),
        "collection" => Some(AppPluginKind::ManageCollection),
        _ => None,
    }
}

/// Download and unpack one item into the sandbox's staging folder for it.
///
/// The item's own subscription row supplies the release; the sandbox supplies
/// the loader and game version a rule may match on. Nothing is written into the
/// game folder — that is [`deploy_sandbox`]'s job, and keeping the two apart is
/// what lets a failed download leave the game untouched.
pub async fn stage_mod(
    db: &LibraryDb,
    sandbox: &Sandbox,
    member: &SandboxMod,
    entry: &LibraryEntry,
    ctx: &SandboxCtx<'_>,
) -> StageOutcome {
    if !entry.installable {
        return StageOutcome::failed(
            &member.mod_key,
            "This item can no longer be installed by the app.",
        );
    }

    if entry.file_url.is_none() {
        return StageOutcome::failed(&member.mod_key, "This item has no release file yet.");
    }

    let Some(slug) = sandbox.app_slug.as_deref().or(entry.app_slug.as_deref()) else {
        return StageOutcome::failed(&member.mod_key, "This game has no plugin folder.");
    };

    let Some(kind) = rule_kind(&entry.kind) else {
        return StageOutcome::failed(&member.mod_key, "That is not something a sandbox holds.");
    };

    let file_name = super::install::safe_file_name(entry);

    let Some(rule) = ctx
        .plugins
        .choose(slug, kind, &file_name, sandbox.loader.as_deref())
    else {
        return StageOutcome::failed(
            &member.mod_key,
            "No install rule for this game — the app cannot place its files.",
        );
    };

    let Some(manage) = &rule.manage else {
        return StageOutcome::failed(&member.mod_key, "That rule declares no install steps.");
    };

    // The staging folder is this run's `gameDir`. Cleared first, so a re-stage
    // at a different version does not leave the old jar beside the new one —
    // the classic `cool-1.3.jar` + `cool-1.4.jar` crash.
    let stage = deploy::stage_dir(ctx.staging_root, sandbox.id, &member.mod_key);

    let _ = std::fs::remove_dir_all(&stage);

    if let Err(e) = std::fs::create_dir_all(&stage) {
        return StageOutcome::failed(
            &member.mod_key,
            format!("Could not prepare the staging folder: {e}"),
        );
    }

    let mut settings = ctx.settings.clone();

    settings.game_dirs.insert(
        entry.app_id.unwrap_or(sandbox.app_id).to_string(),
        stage.display().to_string(),
    );

    let manifest = rule.as_manifest();

    /*
     * `pluginData` is scoped per sandbox and per mod. Two sandboxes staging
     * the same mod run identical steps with an identical `{fileName}`, so a
     * shared scratch directory means the second run's download lands on the
     * first's — and a sandbox pinned to an older release quietly gets the newer
     * file.
     */
    let scope = format!("sb{}-{}", sandbox.id, member.mod_key);

    let jail = match jail_for_scoped(
        &manifest,
        ctx.roots,
        &settings,
        entry.app_id.or(Some(sandbox.app_id)),
        Some(&scope),
    ) {
        Ok(jail) => jail,
        Err(e) => {
            let _ = db.sandbox_mark_stage_failed(sandbox.id, &member.mod_key, &e.to_string());

            return StageOutcome::failed(&member.mod_key, e.to_string());
        }
    };

    let executor = Executor {
        manifest: &manifest,
        jail: &jail,
        http: ctx.http,
        audit: ctx.audit,
        downloads: ctx.downloads,
    };

    audit!(
        ctx.audit,
        Info,
        Install,
        "sandbox.stage.start",
        format!("{} → {} ({})", entry.name, sandbox.name, rule.source),
        plugin = rule.source
    );

    let run_ctx = stage_context(entry, sandbox, &file_name);

    let report = executor.run(&manage.install, &run_ctx).await;

    if report.ok {
        let _ = db.sandbox_mark_staged(
            sandbox.id,
            &member.mod_key,
            entry.latest_release_id,
            entry.latest_version.as_deref(),
        );

        audit!(
            ctx.audit,
            Info,
            Install,
            "sandbox.stage.ok",
            format!("{} → {}", entry.name, sandbox.name),
            plugin = rule.source
        );
    } else {
        let message = report
            .error
            .clone()
            .unwrap_or_else(|| "Staging did not finish.".into());

        let _ = db.sandbox_mark_stage_failed(sandbox.id, &member.mod_key, &message);

        audit!(
            ctx.audit,
            Error,
            Install,
            "sandbox.stage.fail",
            format!("{}: {message}", entry.name),
            plugin = rule.source
        );
    }

    StageOutcome {
        mod_key: member.mod_key.clone(),
        ok: report.ok,
        rule: Some(rule.source.clone()),
        files: report.applied.len(),
        error: report.error.clone(),
    }
}

/// The placeholder table a staging run interpolates.
fn stage_context(entry: &LibraryEntry, sandbox: &Sandbox, file_name: &str) -> RunContext {
    let mut map = std::collections::HashMap::new();

    map.insert("id".into(), entry.id.clone());
    map.insert("kind".into(), entry.kind.clone());
    map.insert("itemId".into(), entry.item_id.to_string());
    map.insert("itemName".into(), entry.name.clone());
    map.insert("webUrl".into(), entry.web_url.clone());
    map.insert("fileName".into(), file_name.to_string());

    if let Some(url) = &entry.file_url {
        map.insert("fileUrl".into(), url.clone());
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

    // The sandbox's own facts, under the names the pre-sandbox rules already
    // use — a rule written for `{installName}` keeps working.
    map.insert("installName".into(), sandbox.name.clone());
    map.insert("sandboxName".into(), sandbox.name.clone());
    map.insert("environment".into(), sandbox.environment.as_str().into());

    if let Some(version) = &sandbox.game_version {
        map.insert("gameVersion".into(), version.clone());
    }
    if let Some(loader) = &sandbox.loader {
        map.insert("loader".into(), loader.clone());
    }

    RunContext(map)
}

/// Delete one mod's staging folder.
pub fn unstage_mod(db: &LibraryDb, sandbox_id: i64, mod_key: &str, ctx: &SandboxCtx<'_>) {
    /*
     * An imported mod's files are NOT this sandbox's to delete. They live in
     * the device's local store and are very likely in another sandbox as well,
     * so removing one from a sandbox removes the row and leaves the files —
     * which is also what makes putting it back a click rather than a re-import.
     * Deleting an import for good is `local_delete`, which is a separate
     * decision and says so.
     */
    if crate::local::LocalMod::id_from_key(mod_key).is_none() {
        let stage = deploy::stage_dir(ctx.staging_root, sandbox_id, mod_key);

        let _ = std::fs::remove_dir_all(stage);
    }

    let _ = db.sandbox_clear_staged(sandbox_id, mod_key);
}

/// The mods a deploy will consider, in priority order.
///
/// Only the ones that are enabled AND staged. A mod that is in the list but has
/// never downloaded contributes nothing and must not be treated as a conflict
/// or as an empty mod — it is simply not ready yet, which the UI shows on the
/// row itself.
pub fn deployable(sandbox: &Sandbox, ctx: &SandboxCtx<'_>) -> Vec<deploy::DeployMod> {
    sandbox
        .enabled_mods()
        .filter(|m| m.staged_at.is_some())
        .map(|m| deploy::DeployMod {
            key: m.mod_key.clone(),
            name: m.name.clone(),
            root: mod_root(sandbox.id, &m.mod_key, ctx),
            priority: m.priority,
        })
        .collect()
}

/// The folder holding one member's files, laid out as they belong under the
/// game directory.
///
/// The one place the two sources of a mod meet, and deliberately the ONLY one:
/// everything downstream — the merge tree, the conflict report, the ledger,
/// the purge's "is this still ours" check — takes a root and does not ask where
/// it came from. An imported mod therefore conflicts with a subscribed one in
/// the same report, in the same load order, with no second code path to keep
/// in step.
fn mod_root(sandbox_id: i64, mod_key: &str, ctx: &SandboxCtx<'_>) -> PathBuf {
    match crate::local::LocalMod::id_from_key(mod_key) {
        Some(id) => crate::local::store::local_root(ctx.local_root, id),
        None => deploy::stage_dir(ctx.staging_root, sandbox_id, mod_key),
    }
}

/// Put a sandbox in front of the game.
pub fn deploy_sandbox(
    db: &LibraryDb,
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
    dry_run: bool,
) -> AppResult<DeployReport> {
    let target = target_dir(sandbox, ctx.settings)?;

    /*
     * The game's own rules, when it has a `sandbox.json`. A game declaring
     * kernel anti-cheat, or simply not supporting a mechanism, is refused HERE
     * rather than after the deploy has half-linked a folder that EAC is about
     * to look at.
     */
    if let Some(slug) = &sandbox.app_slug {
        if let Some(spec) = ctx.plugins.sandbox_spec(slug) {
            if !spec.deploy.allows(sandbox.strategy.as_str()) {
                return Err(AppError::invalid(format!(
                    "{} does not support {} deployment. Supported here: {}.",
                    sandbox.app_name.as_deref().unwrap_or("This game"),
                    sandbox.strategy.as_str(),
                    if spec.deploy.supported_strategies.is_empty() {
                        "any".to_string()
                    } else {
                        spec.deploy.supported_strategies.join(", ")
                    }
                )));
            }
        }
    }

    let previous = db.sandbox_ledger(sandbox.id)?;
    let mods = deployable(sandbox, ctx);

    let staging = deploy::stage_root(ctx.staging_root, sandbox.id);
    let backups = deploy::backup_root(ctx.backup_root, sandbox.id);

    std::fs::create_dir_all(&staging)?;

    let blob = deploy::vfs_blob(ctx.staging_root, sandbox.id);

    let report = deploy::deploy(&DeployRequest {
        strategy: sandbox.strategy,
        target: &target,
        staging: &staging,
        backup_root: &backups,
        vfs_blob: Some(&blob),
        mods: &mods,
        previous: &previous,
        dry_run,
    })?;

    if dry_run {
        return Ok(report);
    }

    /*
     * The ledger is written even when the deploy reported errors. It describes
     * what is ON DISK, and a partial deploy put files there — losing the record
     * of them would leave exactly the orphans the ledger exists to prevent.
     */
    db.sandbox_set_ledger(sandbox.id, &report.ledger)?;
    db.sandbox_mark_deployed(sandbox.id, &serde_json::to_value(&report)?)?;

    audit!(
        ctx.audit,
        Security,
        Install,
        "sandbox.deploy",
        format!(
            "{} → {} ({}; {} placed, {} reused, {} removed, {} backed up)",
            sandbox.name,
            target.display(),
            report.used,
            report.placed,
            report.reused,
            report.removed,
            report.backed_up
        )
    );

    Ok(report)
}

// ------------------------------------------------------------------- Launch

/// What launching a sandbox would run.
///
/// The sandbox's own game folder becomes the jail's `gameDir`, exactly as an
/// install's directory does — the same mechanism rather than a second one, so a
/// profile can never end up with laxer path checks than the main install.
///
/// The sandbox's `launchArgs` and `launchEnv` are appended AFTER the rule's,
/// for the reason a game's own argument parser gives: the last occurrence of a
/// repeated flag wins, so appending is what makes an override override.
pub fn launch_plan(
    db: &LibraryDb,
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
) -> AppResult<LaunchPlan> {
    let target = target_dir(sandbox, ctx.settings)?;

    let slug = sandbox.app_slug.as_deref().ok_or_else(|| {
        AppError::invalid("This sandbox's game has no slug, so no launch rule can be found for it.")
    })?;

    let rule = ctx
        .plugins
        .launch_for(slug, sandbox.loader.as_deref())
        .ok_or_else(|| {
            AppError::invalid(
                "This game has no launch rule, so the app cannot start it. Start it the way you \
                 normally would — the sandbox is deployed either way.",
            )
        })?;

    /*
     * The rule's jail is anchored at the SANDBOX's folder, which is why the
     * settings copy is patched rather than read: a sandbox with its own
     * directory is a different game folder from the app-wide one, and resolving
     * the executable against the wrong one would either fail or — worse —
     * launch the unmodded main install.
     */
    let mut settings = ctx.settings.clone();

    settings
        .game_dirs
        .insert(sandbox.app_id.to_string(), target.display().to_string());

    let manifest = rule.as_manifest();
    let jail = jail_for(&manifest, ctx.roots, &settings, Some(sandbox.app_id))?;

    let options: LaunchOptions = serde_json::from_value(serde_json::Value::Object(
        sandbox.options.clone().into_iter().collect(),
    ))
    .unwrap_or_default();

    /*
     * The virtual tree is attached only when the sandbox is BOTH set to that
     * strategy and has actually been deployed. A blob left over from a strategy
     * change would otherwise be carried into the game, mapping files from a
     * deploy that has since been undone.
     */
    let vfs = if sandbox.strategy == Strategy::Usvfs && sandbox.deployed_at.is_some() {
        let blob = deploy::vfs_blob(ctx.staging_root, sandbox.id);

        if !blob.is_file() {
            return Err(AppError::invalid(
                "This sandbox deploys virtually but has no published filesystem. Deploy it again \
                 before launching.",
            ));
        }

        Some(VfsHandoff {
            blob: blob.display().to_string(),
            root: target.display().to_string(),
        })
    } else {
        None
    };

    let launch_ctx = LaunchContext {
        install_name: Some(sandbox.name.clone()),
        game_version: sandbox.game_version.clone(),
        loader: sandbox.loader.clone(),
        install_dir: Some(target.clone()),
        game_dir: Some(target),
        vfs,
    };

    let mut plan = crate::launch::plan(rule, &jail, &options, &launch_ctx)?;

    for arg in &sandbox.launch_args {
        // The same refusal the rule's own arguments get. A user-typed argument
        // is not more trusted than a plugin-supplied one.
        if arg.contains('\0') || arg.contains('\n') || arg.contains('\r') {
            return Err(AppError::jail(
                "A launch argument contains a control character.",
            ));
        }

        plan.args.push(arg.clone());
    }

    for (key, value) in &sandbox.launch_env {
        if key.contains('\0') || value.contains('\0') {
            return Err(AppError::jail(
                "A launch environment value contains a NUL byte.",
            ));
        }

        plan.env.insert(key.clone(), value.clone());
    }

    /*
     * Advisory, and deliberately not an error. A user who wants to start the
     * game before deploying is entitled to; what they are not entitled to is
     * doing it without being told, because the symptom — a game with none of
     * the sandbox's mods — looks exactly like the mods being broken.
     */
    if sandbox.needs_deploy(&db.sandbox_ledger(sandbox.id)?) {
        audit!(
            ctx.audit,
            Warn,
            Install,
            "sandbox.launch.stale",
            format!("{} has changes that are not deployed", sandbox.name)
        );
    }

    Ok(plan)
}

/// Take a sandbox back out of the game folder.
pub fn purge_sandbox(
    db: &LibraryDb,
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
) -> AppResult<PurgeReport> {
    let target = target_dir(sandbox, ctx.settings)?;
    let previous = db.sandbox_ledger(sandbox.id)?;

    let report = deploy::purge(&target, &previous);

    /*
     * The published tree goes too, unconditionally rather than only for a
     * sandbox currently set to `usvfs`. A user who deployed virtually, switched
     * the strategy and then undeployed would otherwise leave a blob behind that
     * the next launch of that game would still be handed.
     */
    if let Err(e) = deploy::purge_vfs(&deploy::vfs_blob(ctx.staging_root, sandbox.id)) {
        tracing::warn!("could not remove the virtual tree for {}: {e}", sandbox.id);
    }

    /*
     * Only the rows that were actually dealt with are dropped. A file left in
     * place because the user edited it is still a file we put there, so its
     * ledger row survives — otherwise the next deploy would treat it as
     * somebody else's file and move it into the backup store.
     */
    let kept: std::collections::HashSet<&str> = report.kept.iter().map(String::as_str).collect();

    let remaining: Vec<LedgerEntry> = previous
        .into_iter()
        .filter(|e| kept.contains(e.path.as_str()))
        .collect();

    db.sandbox_set_ledger(sandbox.id, &remaining)?;

    if remaining.is_empty() {
        db.sandbox_mark_purged(sandbox.id)?;
    }

    audit!(
        ctx.audit,
        Security,
        Install,
        "sandbox.purge",
        format!(
            "{} ← {} ({} removed, {} restored, {} left alone)",
            sandbox.name,
            target.display(),
            report.removed,
            report.restored,
            report.kept.len()
        )
    );

    Ok(report)
}

/// Check a sandbox's deployment against the disk.
pub fn verify_sandbox(
    db: &LibraryDb,
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
) -> AppResult<deploy::VerifyReport> {
    let target = target_dir(sandbox, ctx.settings)?;
    let ledger = db.sandbox_ledger(sandbox.id)?;

    Ok(deploy::verify(&target, &ledger))
}

/// Which deployment strategies this sandbox could actually use here.
///
/// Both halves have to agree: the machine has to be able to make the link (the
/// engine's probe) and the game has to permit the mechanism (its `sandbox.json`).
pub fn strategies_for(
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
) -> AppResult<Vec<deploy::StrategyReport>> {
    let target = target_dir(sandbox, ctx.settings)?;
    let staging = deploy::stage_root(ctx.staging_root, sandbox.id);

    std::fs::create_dir_all(&staging)?;

    let mut listed = deploy::available_strategies(&staging, &target);

    let spec = sandbox
        .app_slug
        .as_deref()
        .and_then(|slug| ctx.plugins.sandbox_spec(slug));

    if let Some(spec) = spec {
        for report in &mut listed {
            if !spec.deploy.allows(&report.strategy) {
                report.available = false;
                report.reason = Some(format!(
                    "{} does not support this method.",
                    sandbox.app_name.as_deref().unwrap_or("This game")
                ));
            } else if spec.deploy.kernel_anti_cheat()
                && Strategy::parse(&report.strategy).is_some_and(|s| !s.modifies_game_files())
            {
                /*
                 * Still offered, but never silently. Kernel anti-cheat watches
                 * for a game folder whose files are not where they should be,
                 * and the cost of being wrong is somebody's account rather than
                 * a failed install — so the warning rides on the option itself.
                 */
                report.reason = Some(format!(
                    "{} uses kernel-level anti-cheat. Linked files can be treated as \
                     tampering — Direct is the safe choice.",
                    sandbox.app_name.as_deref().unwrap_or("This game")
                ));
            }
        }
    }

    Ok(listed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::sandbox::{Environment, NewSandbox};
    use std::collections::BTreeMap;

    fn settings_with(app_id: i64, dir: &Path) -> AppSettings {
        let mut settings = AppSettings::default();

        settings
            .game_dirs
            .insert(app_id.to_string(), dir.display().to_string());

        settings
    }

    fn new_sandbox() -> NewSandbox {
        NewSandbox {
            app_id: 1,
            app_slug: Some("minecraft".into()),
            app_name: Some("Minecraft".into()),
            name: "Test".into(),
            description: None,
            environment: Environment::Client,
            strategy: Strategy::Direct,
            game_version: None,
            loader: None,
            preset: None,
            game_dir: None,
            options: BTreeMap::new(),
            cloud_sync: false,
            auto_update: true,
        }
    }

    #[test]
    fn a_sandbox_without_a_game_folder_says_so_rather_than_guessing() {
        let db = LibraryDb::open_memory().expect("db");
        let id = db.sandbox_create(&new_sandbox()).expect("create");
        let sandbox = db.sandbox_get(id).expect("get").expect("present");

        let err = target_dir(&sandbox, &AppSettings::default()).expect_err("must refuse");

        assert!(err.to_string().contains("no game folder"));
    }

    #[test]
    fn a_sandbox_folder_overrides_the_apps_configured_one() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let shared = tmp.path().join("shared");
        let own = tmp.path().join("own");

        std::fs::create_dir_all(&shared).expect("mkdir");
        std::fs::create_dir_all(&own).expect("mkdir");

        let db = LibraryDb::open_memory().expect("db");

        let mut new = new_sandbox();
        new.game_dir = Some(own.display().to_string());

        let id = db.sandbox_create(&new).expect("create");
        let sandbox = db.sandbox_get(id).expect("get").expect("present");

        let resolved = target_dir(&sandbox, &settings_with(1, &shared)).expect("resolved");

        assert_eq!(resolved, own);
    }

    #[test]
    fn a_configured_folder_that_is_gone_is_an_error() {
        let db = LibraryDb::open_memory().expect("db");
        let id = db.sandbox_create(&new_sandbox()).expect("create");
        let sandbox = db.sandbox_get(id).expect("get").expect("present");

        let settings = settings_with(1, Path::new("/definitely/not/here"));

        assert!(target_dir(&sandbox, &settings).is_err());
    }

    /// A mod in the list but not yet downloaded is not deployable, and must not
    /// register as an empty mod or as a conflict.
    #[test]
    fn only_staged_and_enabled_mods_are_deployable() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let db = LibraryDb::open_memory().expect("db");

        let id = db.sandbox_create(&new_sandbox()).expect("create");

        let staged = db.sandbox_add_mod(id, "mod", 1, "Staged").expect("add");
        let unstaged = db.sandbox_add_mod(id, "mod", 2, "Not yet").expect("add");
        let disabled = db.sandbox_add_mod(id, "mod", 3, "Off").expect("add");

        db.sandbox_mark_staged(id, &staged, Some(1), None)
            .expect("staged");
        db.sandbox_mark_staged(id, &disabled, Some(1), None)
            .expect("staged");
        db.sandbox_set_mod_enabled(id, &disabled, false)
            .expect("disable");

        let sandbox = db.sandbox_get(id).expect("get").expect("present");

        let audit = Audit::new(tmp.path().join("audit.jsonl"));
        let roots = JailRoots {
            data: tmp.path().join("data"),
            cache: tmp.path().join("cache"),
        };
        let http = reqwest::Client::new();
        let plugins = AppPlugins::default();
        let settings = AppSettings::default();

        let ctx = SandboxCtx {
            plugins: &plugins,
            roots: &roots,
            settings: &settings,
            http: &http,
            audit: &audit,
            downloads: None,
            staging_root: &tmp.path().join("staging"),
            backup_root: &tmp.path().join("backups"),
            local_root: &tmp.path().join("local-mods"),
        };

        let mods = deployable(&sandbox, &ctx);

        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].key, staged);
        assert_ne!(mods[0].key, unstaged);
    }
}
