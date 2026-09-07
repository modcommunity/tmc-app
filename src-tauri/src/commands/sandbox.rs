//! The sandbox surface: create a profile, put mods in it, put it in front of
//! the game.
//!
//! The rules from `commands/mod.rs` hold, and one more that is specific to this
//! file and load-bearing: **the frontend never names a directory to deploy
//! into.** It names a sandbox by id; Rust looks up that sandbox's folder, or
//! the app's configured folder for the game, and everything below resolves
//! through the jail from there. A command that took a target path would make
//! every check in `anchor` and `deploy` advisory — an injected script in a mod
//! description could point a deploy at anything.
//!
//! The same applies to staging: a staging folder is derived from the sandbox
//! id, never supplied.

use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use tmc_core::audit;
use tmc_core::deploy::{PurgeReport, StrategyReport, VerifyReport};
use tmc_core::error::{AppError, AppResult};
use tmc_core::launch::{describe, LaunchPlan};
use tmc_core::library::autoupdate::{self, AutoUpdateReport, Outdated};
use tmc_core::library::dependency::{DependencyReport, Edge, Relation};
use tmc_core::library::deploy::{
    deploy_sandbox, launch_plan, purge_sandbox, stage_mod, strategies_for, verify_sandbox,
    StageOutcome,
};
use tmc_core::library::sandbox::{NewSandbox, Sandbox, SandboxPatch};
use tmc_core::plugins::apps::SandboxSpec;

use crate::commands::library::LaunchPreview;
use crate::state::AppState;

/// A sandbox plus the answers every screen showing one wants.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxRow {
    #[serde(flatten)]
    pub sandbox: Sandbox,
    /// Has anything changed since the last deploy?
    pub needs_deploy: bool,
    /// Files this sandbox currently has in the game folder.
    pub deployed_files: usize,
    /// The folder it deploys into, or `None` when none is configured yet — the
    /// UI's cue to ask for one before anything else.
    pub target_dir: Option<String>,
}

fn row(sandbox: Sandbox, state: &AppState) -> AppResult<SandboxRow> {
    let ledger = state.library.sandbox_ledger(sandbox.id)?;
    let settings = state.settings.get();

    Ok(SandboxRow {
        needs_deploy: sandbox.needs_deploy(&ledger),
        deployed_files: ledger.len(),
        target_dir: tmc_core::library::deploy::target_dir(&sandbox, &settings)
            .ok()
            .map(|p| p.to_string_lossy().into_owned()),
        sandbox,
    })
}

/// Every sandbox, or one game's.
#[tauri::command]
pub fn sandbox_list(state: State<'_, AppState>, app_id: Option<i64>) -> AppResult<Vec<SandboxRow>> {
    state
        .library
        .sandbox_list(app_id)?
        .into_iter()
        .map(|s| row(s, &state))
        .collect()
}

#[tauri::command]
pub fn sandbox_get(state: State<'_, AppState>, id: i64) -> AppResult<Option<SandboxRow>> {
    match state.library.sandbox_get(id)? {
        Some(sandbox) => Ok(Some(row(sandbox, &state)?)),
        None => Ok(None),
    }
}

/// Create one, optionally from one of the game's presets.
///
/// The preset is applied HERE rather than in the webview: a preset carries a
/// deployment strategy and a set of option values, and letting the frontend
/// assemble those would mean a sandbox whose settings never went through the
/// game's own schema.
#[tauri::command]
pub async fn sandbox_create(
    state: State<'_, AppState>,
    mut new: NewSandbox,
    preset: Option<String>,
) -> AppResult<SandboxRow> {
    /*
     * A sandbox with no slug has no game rules at all — no preset, no strategy
     * validation, no install rule and no launch rule. It is created
     * successfully and then does nothing, which is the worst way for this to
     * fail: nothing on screen is wrong until somebody presses Play.
     *
     * The caller often cannot supply one. A server's page carries an `AppRef`
     * with a name and an id; the Library's game rows are keyed by the id that
     * `settings.gameDirs` uses. So it is resolved here, from what the device
     * knows and then from the catalogue.
     */
    if new.app_slug.as_deref().unwrap_or("").trim().is_empty() {
        new.app_slug = state.app_slug_for(new.app_id);
    }

    if new.app_slug.is_none() {
        if let Ok(answer) = state
            .api
            .request(
                tmc_core::api::Method::GET,
                &format!("/apps?ids={}&limit=1", new.app_id),
                None,
                false,
            )
            .await
        {
            if let Some(app) = answer
                .get("apps")
                .and_then(serde_json::Value::as_array)
                .and_then(|apps| apps.first())
            {
                new.app_slug = app
                    .get("slug")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_ascii_lowercase);

                if new.app_name.is_none() {
                    new.app_name = app
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                }

                if let Some(slug) = &new.app_slug {
                    state.remember_app_ids([(slug.clone(), new.app_id)]);
                }
            }
        }
    }

    let plugins = state.app_plugins();

    let spec = new
        .app_slug
        .as_deref()
        .and_then(|slug| plugins.sandbox_spec(slug));

    if let (Some(spec), Some(preset_id)) = (spec, preset.as_deref()) {
        let chosen = spec
            .preset(preset_id)
            .ok_or_else(|| AppError::invalid("That preset does not exist for this game."))?;

        new.preset = Some(chosen.id.clone());

        if let Some(env) = chosen
            .environment
            .as_deref()
            .and_then(tmc_core::library::sandbox::Environment::parse)
        {
            new.environment = env;
        }

        if let Some(strategy) = chosen
            .strategy
            .as_deref()
            .and_then(tmc_core::deploy::Strategy::parse)
        {
            new.strategy = strategy;
        }

        if chosen.game_version.is_some() {
            new.game_version = chosen.game_version.clone();
        }

        if chosen.loader.is_some() {
            new.loader = chosen.loader.clone();
        }

        // The schema's defaults first, then the preset's — so a preset that
        // sets one option does not silently clear the rest.
        let mut options = spec.default_options();

        options.extend(chosen.options.clone());

        new.options = options;
    }

    // A strategy the game says it does not support is refused at creation
    // rather than at deploy, when forty mods have already been assigned.
    if let Some(spec) = spec {
        if !spec.deploy.allows(new.strategy.as_str()) {
            return Err(AppError::invalid(format!(
                "This game does not support {} deployment.",
                new.strategy.as_str()
            )));
        }

        new.options = spec.clamp_options(&new.options);
    }

    let id = state.library.sandbox_create(&new)?;

    tmc_core::audit!(
        state.audit,
        Info,
        App,
        "sandbox.create",
        format!(
            "{} ({}, {})",
            new.name,
            new.environment.as_str(),
            new.strategy.as_str()
        )
    );

    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::internal("the sandbox vanished after being created"))?;

    row(sandbox, &state)
}

#[tauri::command]
pub fn sandbox_patch(
    state: State<'_, AppState>,
    id: i64,
    mut patch: SandboxPatch,
) -> AppResult<SandboxRow> {
    let existing = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();

    let spec = existing
        .app_slug
        .as_deref()
        .and_then(|slug| plugins.sandbox_spec(slug));

    if let Some(spec) = spec {
        if let Some(strategy) = patch.strategy {
            if !spec.deploy.allows(strategy.as_str()) {
                return Err(AppError::invalid(format!(
                    "This game does not support {} deployment.",
                    strategy.as_str()
                )));
            }
        }

        // Clamped against the game's own schema, so a value the webview sent
        // cannot reach a command line unbounded.
        if let Some(options) = &patch.options {
            patch.options = Some(spec.clamp_options(options));
        }
    }

    /*
     * A game folder set here goes through the anchor validator, exactly as a
     * global game directory does. It is the anchor of the jail for every
     * install into this sandbox, so it gets no more trust for having arrived on
     * a different command.
     */
    if let Some(Some(dir)) = &patch.game_dir {
        let canonical = tmc_core::anchor::validate_root(dir, &protected(&state))?;

        patch.game_dir = Some(Some(canonical.to_string_lossy().into_owned()));

        tmc_core::audit!(
            state.audit,
            Security,
            App,
            "sandbox.game_dir",
            format!("{} → {}", existing.name, canonical.display())
        );
    }

    state.library.sandbox_patch(id, &patch)?;

    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::internal("the sandbox vanished after being patched"))?;

    row(sandbox, &state)
}

/// Delete a sandbox, taking its deployment out of the game folder first.
///
/// `keep_files` leaves the staging folder alone, which is what somebody
/// rebuilding a profile wants — the mods are already downloaded.
#[tauri::command]
pub fn sandbox_delete(
    state: State<'_, AppState>,
    id: i64,
    keep_files: Option<bool>,
) -> AppResult<PurgeReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let dirs = state.sandbox_dirs();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &dirs);

    /*
     * Undeployed BEFORE the row is deleted. The ledger is the only record of
     * which files in the game folder are ours, and it goes with the row — so a
     * delete that skipped this would strand every file the sandbox placed, with
     * nothing left that knows how to remove them.
     */
    let report = purge_sandbox(&state.library, &sandbox, &ctx).unwrap_or_default();

    if !keep_files.unwrap_or(false) {
        // Staging only. The device's imported mods are NOT this sandbox's to
        // delete — one import is routinely in several sandboxes, and deleting
        // an import for good is its own command.
        let _ = std::fs::remove_dir_all(tmc_core::deploy::stage_root(&dirs.staging, id));
    }

    state.library.sandbox_delete(id)?;

    tmc_core::audit!(
        state.audit,
        Info,
        App,
        "sandbox.delete",
        format!("{} ({} files removed)", sandbox.name, report.removed)
    );

    Ok(report)
}

#[tauri::command]
pub fn sandbox_set_default(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    state.library.sandbox_set_default(id)
}

// ------------------------------------------------------------------ Contents

/// Add a subscribed item to a sandbox.
///
/// The name comes from the LIBRARY rather than from the caller: the webview
/// naming an item would let a rendered mod description put a row in somebody's
/// sandbox list under any label it liked.
#[tauri::command]
pub async fn sandbox_add_mod(
    state: State<'_, AppState>,
    id: i64,
    kind: String,
    item_id: i64,
) -> AppResult<SandboxRow> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let entry = state.library.find_item(&kind, item_id)?.ok_or_else(|| {
        AppError::invalid(
            "Subscribe to this item first — the app only installs things your account asked \
                 it to keep.",
        )
    })?;

    if entry.app_id.is_some_and(|app| app != sandbox.app_id) {
        return Err(AppError::invalid(
            "That item is for a different game than this sandbox.",
        ));
    }

    state
        .library
        .sandbox_add_mod(id, &kind, item_id, &entry.name)?;

    /*
     * The item's dependency edges are fetched HERE, once, and cached. The
     * alternative is asking the API per item at deploy time, which is forty
     * requests on a screen somebody is waiting on for data that changes about
     * as often as a mod is re-released.
     *
     * A failure is not fatal: the item is already in the sandbox, and the
     * report names anything it could not check rather than pretending it found
     * nothing.
     */
    if let Err(err) = cache_dependencies(&state, &kind, item_id).await {
        let detail = err.detail();

        tracing::warn!("could not fetch dependencies for {kind}:{item_id}: {detail}");
    }

    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::internal("the sandbox vanished"))?;

    row(sandbox, &state)
}

/// What this sandbox's items say about each other.
///
/// A local query — every edge was cached when its item was added — so a screen
/// can call it on every render without a request going anywhere.
#[tauri::command]
pub fn sandbox_check(state: State<'_, AppState>, id: i64) -> AppResult<DependencyReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    state.library.dependency_check(&sandbox)
}

/// Re-fetch the edges for everything in a sandbox.
///
/// What the "check again" button calls. Bounded by the sandbox's own size, and
/// one failure does not stop the rest — an item whose page 404s should not cost
/// the other thirty their answer.
#[tauri::command]
pub async fn sandbox_refresh_dependencies(
    state: State<'_, AppState>,
    id: i64,
) -> AppResult<DependencyReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    for member in &sandbox.mods {
        if let Err(err) = cache_dependencies(&state, &member.kind, member.item_id).await {
            tracing::warn!(
                "could not refresh dependencies for {}: {}",
                member.mod_key,
                err.detail()
            );
        }
    }

    state.library.dependency_check(&sandbox)
}

/// Add everything a sandbox is missing, where the account is subscribed to it.
///
/// **Only subscribed items.** A missing dependency the user has not subscribed
/// to cannot be materialised — the installer refuses an item the account did
/// not ask to keep — so adding the row would produce a sandbox entry that can
/// never download. The UI links those out to their pages instead.
#[tauri::command]
pub async fn sandbox_add_missing(state: State<'_, AppState>, id: i64) -> AppResult<Vec<String>> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let report = state.library.dependency_check(&sandbox)?;

    let mut added = Vec::new();

    for edge in report.missing {
        let Some(entry) = state.library.find_item(&edge.rel_kind, edge.rel_id)? else {
            continue;
        };

        if state
            .library
            .sandbox_add_mod(id, &edge.rel_kind, edge.rel_id, &entry.name)
            .is_ok()
        {
            let _ = cache_dependencies(&state, &edge.rel_kind, edge.rel_id).await;

            added.push(entry.name);
        }
    }

    Ok(added)
}

/// Fetch one item's dependency edges from the API and cache them.
///
/// The parsing is deliberately forgiving: a field the server adds later must
/// not make the whole item uncheckable, and an edge whose relation this build
/// does not recognise is dropped rather than guessed at — guessing would mean
/// inventing a requirement or a conflict out of a string.
async fn cache_dependencies(state: &AppState, kind: &str, item_id: i64) -> AppResult<()> {
    let path = format!("/content/{kind}/{item_id}");

    let value = state
        .api
        .request(tmc_core::api::Method::GET, &path, None, false)
        .await?;

    let edges: Vec<Edge> = value
        .get("dependencies")
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|raw| {
                    Some(Edge {
                        kind: kind.to_string(),
                        item_id,
                        rel_kind: raw.get("kind")?.as_str()?.to_string(),
                        rel_id: raw.get("id")?.as_i64()?,
                        relation: Relation::parse(raw.get("relation")?.as_str()?)?,
                        name: raw
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("Unknown")
                            .to_string(),
                        icon: raw
                            .get("icon")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        note: raw
                            .get("note")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    state.library.dependency_set(kind, item_id, &edges)
}

#[tauri::command]
pub fn sandbox_remove_mod(
    state: State<'_, AppState>,
    id: i64,
    mod_key: String,
) -> AppResult<SandboxRow> {
    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let dirs = state.sandbox_dirs();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &dirs);

    tmc_core::library::deploy::unstage_mod(&state.library, id, &mod_key, &ctx);

    state.library.sandbox_remove_mod(id, &mod_key)?;

    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    row(sandbox, &state)
}

#[tauri::command]
pub fn sandbox_set_mod_enabled(
    state: State<'_, AppState>,
    id: i64,
    mod_key: String,
    enabled: bool,
) -> AppResult<()> {
    state.library.sandbox_set_mod_enabled(id, &mod_key, enabled)
}

/// Rewrite the load order, first to last.
#[tauri::command]
pub fn sandbox_reorder(
    state: State<'_, AppState>,
    id: i64,
    keys: Vec<String>,
) -> AppResult<Vec<tmc_core::library::sandbox::SandboxMod>> {
    if keys.len() > tmc_core::library::sandbox::MAX_MODS_PER_SANDBOX {
        return Err(AppError::invalid(
            "That is more items than a sandbox holds.",
        ));
    }

    state.library.sandbox_reorder(id, &keys)?;
    state.library.sandbox_mods(id)
}

/// Turn a sandbox into a code somebody can paste.
///
/// Read-only, and it carries item ids rather than files — see
/// `tmc_core::library::share` for what a code holds and, more importantly, what
/// it deliberately does not.
#[tauri::command]
pub fn sandbox_export(state: State<'_, AppState>, id: i64) -> AppResult<String> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    tmc_core::library::share::export(&sandbox)
}

/// What importing a code WOULD do, without doing any of it.
///
/// The same split every other risky operation here has: a code is a string from
/// somebody else, and pressing Import on one should show what is in it before
/// forty subscriptions are made on the account.
#[tauri::command]
pub fn sandbox_import_preview(code: String) -> AppResult<tmc_core::library::share::SharedSandbox> {
    tmc_core::library::share::import(&code)
}

/// What an import actually did.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub sandbox: Option<SandboxRow>,
    pub added: usize,
    /// Items that could not be added, with the reason, one line each.
    pub skipped: Vec<String>,
}

/// Create a sandbox from a code.
///
/// **The items are subscribed, not downloaded.** Each one goes through the
/// account's own `/subscriptions` endpoint and then through `sandbox_add_mod`,
/// which is what keeps the imported profile updating like any other and what
/// stops this becoming a way to put an arbitrary item list on a device without
/// the account ever asking for it.
///
/// **Staging is NOT done here.** A modpack of two hundred mods is two hundred
/// downloads, and starting them inside a command the UI is awaiting would give
/// somebody a frozen dialog and no queue to look at. The sandbox is created and
/// the user presses Deploy, which is the same path every other sandbox takes.
///
/// **An item that cannot be added does not stop the rest.** A code shared by
/// somebody with access to something this account does not, or a mod that has
/// since been withdrawn, is a line in `skipped` — not a failed import of the
/// other hundred and ninety.
#[tauri::command]
pub async fn sandbox_import(
    state: State<'_, AppState>,
    code: String,
    name: Option<String>,
) -> AppResult<ImportReport> {
    let shared = tmc_core::library::share::import(&code)?;

    let mut report = ImportReport::default();

    let created = sandbox_create(
        state.clone(),
        tmc_core::library::sandbox::NewSandbox {
            app_id: shared.app_id,
            app_slug: shared.app_slug.clone(),
            app_name: shared.app_name.clone(),
            name: name
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| shared.name.clone()),
            description: shared.description.clone(),
            environment: shared.environment,
            strategy: shared.strategy,
            game_version: shared.game_version.clone(),
            loader: shared.loader.clone(),
            preset: shared.preset.clone(),
            /*
             * Never from the code. A game folder is an answer about one machine
             * and a jail anchor — `share` does not export one, and refusing to
             * accept one here is the second half of that.
             */
            game_dir: None,
            options: shared.options.clone(),
            /*
             * Both default ON for a fresh sandbox and are NOT taken from the
             * code. They are the importer's preferences about their own
             * account, not the exporter's: whether somebody's mod list leaves
             * their machine, and whether it tracks new releases, are decisions
             * that belong to whoever pressed Import.
             */
            cloud_sync: true,
            auto_update: true,
        },
        None,
    )
    .await?;

    let id = created.sandbox.id;

    /*
     * Items whose subscription failed, so the add loop below does not report
     * the same one twice. Without this a withdrawn mod produces two lines —
     * "could not subscribe" and then "subscribe to this item first" — which
     * reads as two problems with one item.
     */
    let mut failed: std::collections::BTreeSet<(String, i64)> = std::collections::BTreeSet::new();

    for item in &shared.mods {
        /*
         * Subscribed first when the account is not already, exactly as the
         * one-click install does and for the same reason: `sandbox_add_mod`
         * refuses an unsubscribed item, and an import that worked around that
         * would be putting items on a device the account never asked for.
         */
        if state.library.find_item(&item.kind, item.item_id)?.is_some() {
            continue;
        }

        let payload = serde_json::json!({
            "kind": item.kind,
            "itemId": item.item_id,
            "subscribed": true,
        });

        if let Err(err) = state
            .api
            .request(
                tmc_core::api::Method::POST,
                "/subscriptions",
                Some(payload),
                true,
            )
            .await
        {
            report.skipped.push(format!("{}: {}", item.name, err));
            failed.insert((item.kind.clone(), item.item_id));
        }
    }

    /*
     * ONE sync for the whole code rather than one per item. Two hundred items
     * is two hundred subscriptions, and syncing after each would be two hundred
     * round trips for a watermark that moves once.
     */
    if let Err(err) = tmc_core::library::sync_once(&state.api, &state.library, false).await {
        report
            .skipped
            .push(format!("Could not refresh the library: {}", err.detail()));
    }

    for item in &shared.mods {
        if failed.contains(&(item.kind.clone(), item.item_id)) {
            continue;
        }

        match sandbox_add_mod(state.clone(), id, item.kind.clone(), item.item_id).await {
            Ok(_) => {
                report.added += 1;

                // The exporter's own ordering, which is the load order — a
                // modpack whose mods arrive in a different order is a modpack
                // that behaves differently.
                let _ = state.library.sandbox_set_mod_enabled(
                    id,
                    &format!("{}:{}", item.kind, item.item_id),
                    item.enabled,
                );
            }
            Err(err) => report.skipped.push(format!("{}: {}", item.name, err)),
        }
    }

    /*
     * The load order, applied in one pass once every member exists. Doing it
     * per item would reorder a list that is still being built, and the merge
     * tree reads priority — so a wrong order here is a different set of files
     * in the game folder, not a cosmetic difference.
     */
    let keys: Vec<String> = {
        let mut ordered: Vec<_> = shared
            .mods
            .iter()
            .filter(|m| !failed.contains(&(m.kind.clone(), m.item_id)))
            .collect();

        /*
         * ASCENDING, because that is what `sandbox_reorder` means.
         *
         * It assigns `priority = index`, so the first key gets priority 0 and
         * the last gets the highest — and higher priority WINS a contested
         * path in `merge::build`. `sandbox_mods` reads back
         * `ORDER BY priority ASC`, so an exported list is already in this
         * order and sorting it descending reversed it: a pack exported as
         * A, B, C came back as C, B, A and produced a different set of files in
         * the game folder than the exporter had. Which is precisely what the
         * comment below says must not happen.
         */
        ordered.sort_by_key(|m| m.priority);

        ordered
            .iter()
            .map(|m| format!("{}:{}", m.kind, m.item_id))
            .collect()
    };

    if let Err(err) = state.library.sandbox_reorder(id, &keys) {
        report
            .skipped
            .push(format!("Could not apply the load order: {}", err.detail()));
    }

    audit!(
        state.audit,
        Info,
        Install,
        "sandbox.import",
        format!(
            "{} ({} added, {} skipped)",
            created.sandbox.name,
            report.added,
            report.skipped.len()
        )
    );

    report.sandbox = state
        .library
        .sandbox_get(id)?
        .map(|s| row(s, &state))
        .transpose()?;

    Ok(report)
}

/// What a one-click install did, step by step.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickInstallReport {
    /// The account was subscribed to the item as part of this.
    pub subscribed: bool,
    /// It was already in the sandbox and nothing was added.
    pub already_present: bool,
    pub staged: Option<StageOutcome>,
    pub deployed: Option<tmc_core::deploy::DeployReport>,
    /// Steps that did not run, and why. Never fatal on its own.
    pub warnings: Vec<String>,
}

/// **Install one mod or asset into one sandbox**, in a single operation.
///
/// The whole flow a user means by "install this": subscribe if they are not
/// already, wait for the library to know about it, add it to the sandbox, stage
/// its files, and deploy them into the game.
///
/// WHY IT IS ONE COMMAND AND NOT FIVE
/// ---------------------------------
/// Because the alternative is the webview orchestrating it, and the rule this
/// architecture rests on is that the PLAN is executed in Rust. Five commands
/// called in sequence from the frontend is a plan the frontend owns — an
/// injected script in a rendered mod description could run four of them, or run
/// them against a different sandbox, or stop between the stage and the deploy
/// and leave a game folder half-modded with nothing on screen saying so.
///
/// It also makes the audit trail one entry describing one intent rather than
/// five entries somebody has to reassemble.
///
/// EVERY STEP KEEPS ITS OWN CHECKS
/// ------------------------------
/// Nothing is bypassed by being called from here. `sandbox_add_mod` still
/// refuses an item the account is not subscribed to and still refuses one
/// belonging to another game; staging still goes through the game's own rule
/// and the plugin jail; deploying still goes through the ledger. This composes
/// them, it does not shortcut them.
///
/// ASSETS AND MODS ARE THE SAME PATH
/// --------------------------------
/// Deliberately. A resource pack and a jar mod differ only in which app rule
/// matches them (`manage_asset` versus `manage_mod`), and that selection
/// already happens inside the executor. A second command for assets would be a
/// second place for the two to drift.
#[tauri::command]
pub async fn sandbox_install_item(
    state: State<'_, AppState>,
    id: i64,
    kind: String,
    item_id: i64,
    deploy: Option<bool>,
) -> AppResult<QuickInstallReport> {
    let mut report = QuickInstallReport::default();

    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    /*
     * Subscribing first, when the account has not already.
     *
     * `sandbox_add_mod` refuses an unsubscribed item on purpose — the app only
     * materialises what the account asked it to keep — so a one-click install
     * has to make that true rather than work around it. Subscribing is also the
     * thing that makes the mod follow the user to their other devices, which is
     * what somebody pressing Install on a sandbox almost always wants.
     */
    if state.library.find_item(&kind, item_id)?.is_none() {
        let payload = serde_json::json!({
            "kind": kind,
            "itemId": item_id,
            "subscribed": true,
        });

        state
            .api
            .request(
                tmc_core::api::Method::POST,
                "/subscriptions",
                Some(payload),
                true,
            )
            .await?;

        report.subscribed = true;

        /*
         * A sync, because the subscription is the SERVER's fact and the local
         * row is a mirror of it. Without this the row does not exist yet and
         * every step below fails on an item that was just subscribed — which
         * reads as the button not working.
         */
        if let Err(err) = tmc_core::library::sync_once(&state.api, &state.library, false).await {
            report
                .warnings
                .push(format!("Could not refresh the library: {}", err.detail()));
        }
    }

    if state.library.find_item(&kind, item_id)?.is_none() {
        return Err(AppError::invalid(
            "The subscription has not reached this device yet. Try again in a moment.",
        ));
    }

    let already = sandbox
        .mods
        .iter()
        .any(|m| m.kind == kind && m.item_id == item_id);

    report.already_present = already;

    if !already {
        sandbox_add_mod(state.clone(), id, kind.clone(), item_id).await?;
    }

    // Re-read: `sandbox_add_mod` wrote a row, and staging needs the member it
    // created rather than the snapshot taken before it existed.
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox disappeared mid-install."))?;

    let Some(member) = sandbox
        .mods
        .iter()
        .find(|m| m.kind == kind && m.item_id == item_id)
    else {
        return Err(AppError::internal("the item was not added to the sandbox"));
    };

    let entry = state
        .library
        .find_item(&kind, item_id)?
        .ok_or_else(|| AppError::internal("the library row vanished mid-install"))?;

    {
        let plugins = state.app_plugins();
        let settings = state.settings.get();
        let roots = state.jail_roots();
        let dirs = state.sandbox_dirs();

        let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &dirs);

        report.staged = Some(stage_mod(&state.library, &sandbox, member, &entry, &ctx).await);
    }

    let staged_ok = report.staged.as_ref().is_some_and(|s| s.ok);

    /*
     * Deploying only when the staging actually produced files.
     *
     * A deploy after a failed stage writes the sandbox's OTHER mods into the
     * game folder and reports success, which is the worst possible outcome of
     * pressing Install on one mod: the folder changed, the thing asked for is
     * missing, and the report says it worked.
     */
    if deploy.unwrap_or(true) && staged_ok {
        match sandbox_deploy(state.clone(), id, Some(false)) {
            Ok(deployed) => report.deployed = Some(deployed),
            Err(err) => report.warnings.push(err.to_string()),
        }
    } else if deploy.unwrap_or(true) && !staged_ok {
        report
            .warnings
            .push("Nothing was deployed, because the files could not be staged.".into());
    }

    audit!(
        state.audit,
        Info,
        Install,
        "sandbox.quick_install",
        format!("{} → {}", entry.name, sandbox.name)
    );

    Ok(report)
}

// -------------------------------------------------------------------- Staging

/// Download and unpack everything in a sandbox that is not staged yet.
///
/// Returns one outcome per item rather than failing on the first: a mod whose
/// game has no rule must not stop the other forty from staging.
#[tauri::command]
pub async fn sandbox_stage(
    state: State<'_, AppState>,
    id: i64,
    force: Option<bool>,
) -> AppResult<Vec<StageOutcome>> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let dirs = state.sandbox_dirs();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &dirs);

    let force = force.unwrap_or(false);

    let mut out = Vec::new();

    for member in &sandbox.mods {
        if !member.enabled {
            continue;
        }

        // Already staged at the release the server currently offers.
        let entry = state.library.find_item(&member.kind, member.item_id)?;

        let Some(entry) = entry else {
            continue;
        };

        let current = member.staged_at.is_some() && member.release_id == entry.latest_release_id;

        if current && !force {
            continue;
        }

        out.push(stage_mod(&state.library, &sandbox, member, &entry, &ctx).await);
    }

    Ok(out)
}

// ----------------------------------------------------------------- Deployment

/// Put the sandbox in front of the game.
#[tauri::command]
pub fn sandbox_deploy(
    state: State<'_, AppState>,
    id: i64,
    dry_run: Option<bool>,
) -> AppResult<tmc_core::deploy::DeployReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let dirs = state.sandbox_dirs();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &dirs);

    let mut report = deploy_sandbox(&state.library, &sandbox, &ctx, dry_run.unwrap_or(false))?;

    /*
     * Dependency problems ride along as WARNINGS on the deploy report, and the
     * deploy still happens. The metadata is author-written and frequently wrong
     * — a required edge left in place after a mod absorbed its own dependency
     * is the normal state of every mod site — and a refusal with no escape
     * hatch is one people learn to ignore, which costs the accurate warnings
     * their credibility too. See `library::dependency`.
     */
    if let Ok(deps) = state.library.dependency_check(&sandbox) {
        for edge in &deps.missing {
            report.warnings.push(format!(
                "{} is required by something in this sandbox and is not in it.",
                edge.name
            ));
        }

        for clash in &deps.conflicts {
            report.warnings.push(format!(
                "{} and {} are marked as incompatible.",
                clash.a_name, clash.b_name
            ));
        }
    }

    Ok(report)
}

/// Take it back out.
#[tauri::command]
pub fn sandbox_purge(state: State<'_, AppState>, id: i64) -> AppResult<PurgeReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let dirs = state.sandbox_dirs();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &dirs);

    purge_sandbox(&state.library, &sandbox, &ctx)
}

/// Check what the last deploy left behind against what is on disk.
#[tauri::command]
pub fn sandbox_verify(state: State<'_, AppState>, id: i64) -> AppResult<VerifyReport> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let dirs = state.sandbox_dirs();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &dirs);

    verify_sandbox(&state.library, &sandbox, &ctx)
}

// ------------------------------------------------------------------- Launch

/// What starting this sandbox would run, WITHOUT starting it.
///
/// The same contract the install launcher has: the exact program, arguments and
/// working directory are shown first. A declarative launcher is only auditable
/// by the person it affects if they can see what it resolved to.
#[tauri::command]
pub fn sandbox_launch_preview(state: State<'_, AppState>, id: i64) -> AppResult<LaunchPreview> {
    let plan = sandbox_plan(&state, id)?;
    let command = describe(&plan);

    Ok(LaunchPreview {
        plan,
        command,
        session: None,
    })
}

/// Start the game with this sandbox in front of it.
///
/// The plan is re-resolved here rather than taken from the webview, for the
/// reason `launch_install` gives: accepting a caller-supplied plan would make
/// every check in `tmc_core::launch` advisory.
#[tauri::command]
pub fn sandbox_launch(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> AppResult<LaunchPreview> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plan = sandbox_plan(&state, id)?;
    let command = describe(&plan);

    audit!(
        state.audit,
        Security,
        Install,
        "sandbox.launch",
        command.clone(),
        plugin = plan.rule
    );

    /*
     * A sandbox already running must not be started twice.
     *
     * Not a nicety: the second copy of a game opens the same files the first
     * has mapped, and on Windows that is a game that fails to start with an
     * error naming a file the user has never heard of. It also makes the play
     * clock for that sandbox the sum of two overlapping sessions, which is a
     * number that can exceed wall-clock time.
     */
    if state.sessions.running_for_sandbox(id) {
        return Err(AppError::invalid(
            "That sandbox is already running. Close the game first, or stop it from the Library.",
        ));
    }

    let session = crate::spawn::run(
        &app,
        &state,
        &plan,
        tmc_core::session::SessionSpec {
            app_id: Some(sandbox.app_id),
            app_slug: sandbox.app_slug.clone(),
            label: sandbox.name.clone(),
            sandbox_id: Some(id),
            /*
             * The cloud install id, which is what playtime is reported against.
             * A sandbox with `cloudSync` off has none — its play time stays on
             * this device, which is the whole meaning of that switch.
             */
            install_id: sandbox.remote_id,
        },
    )?;

    Ok(LaunchPreview {
        plan,
        command,
        session: Some(session),
    })
}

fn sandbox_plan(state: &State<'_, AppState>, id: i64) -> AppResult<LaunchPlan> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let dirs = state.sandbox_dirs();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &dirs);

    launch_plan(&state.library, &sandbox, &ctx)
}

/// Which deployment strategies would actually work for this sandbox, here.
#[tauri::command]
pub fn sandbox_strategies(state: State<'_, AppState>, id: i64) -> AppResult<Vec<StrategyReport>> {
    let sandbox = state
        .library
        .sandbox_get(id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let dirs = state.sandbox_dirs();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &dirs);

    strategies_for(&sandbox, &ctx)
}

/// Everything with a newer release than the one staged.
///
/// Ignores the auto-update switches on purpose: somebody who turned automatic
/// updates off still wants to be TOLD there is one, and being told is the whole
/// point of turning it off rather than unsubscribing.
#[tauri::command]
pub fn sandbox_updates(state: State<'_, AppState>) -> AppResult<Vec<Outdated>> {
    autoupdate::outdated(&state.library)
}

/// Bring every eligible sandbox forward.
///
/// Called after each library sync and on launch. Redeploys only the sandboxes
/// that were ALREADY deployed — staging is invisible, deploying writes into
/// somebody's game folder, and an automatic pass may not do the second on its
/// own initiative.
#[tauri::command]
pub async fn sandbox_auto_update(state: State<'_, AppState>) -> AppResult<AutoUpdateReport> {
    let plugins = state.app_plugins();
    let settings = state.settings.get();
    let roots = state.jail_roots();
    let dirs = state.sandbox_dirs();

    let ctx = state.sandbox_ctx(&plugins, &settings, &roots, &dirs);

    autoupdate::run(&state.library, &ctx).await
}

/// A game's presets, option schema and deployment rules.
///
/// `None` for a game that ships no `sandbox.json`, which is not an error — it
/// gets the app's defaults, and that is the right answer for the many games
/// where "put the file in `mods/` and link it" is the whole story.
#[tauri::command]
pub fn sandbox_spec(state: State<'_, AppState>, slug: String) -> Option<SandboxSpec> {
    state.app_plugins().sandbox_spec(&slug).cloned()
}

/// Directories a game folder must not be, or contain.
///
/// The app's own data, logs, cache, plugins, staging and backups. A jail
/// anchored above any of them would enclose the plugin registry — and a plugin
/// that can rewrite the registry can grant itself permissions.
fn protected(state: &AppState) -> Vec<PathBuf> {
    vec![
        state.paths.data.clone(),
        state.paths.logs.clone(),
        state.paths.cache.clone(),
        state.paths.plugins.clone(),
        state.paths.staging_dir(),
        state.paths.backup_dir(),
    ]
}
