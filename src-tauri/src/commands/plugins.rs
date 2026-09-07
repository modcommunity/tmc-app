use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use tmc_core::audit;
use tmc_core::error::{AppError, AppResult};
use tmc_core::plugins::manifest::Theme;
use tmc_core::plugins::registry::PluginRecord;
use tmc_core::plugins::signature::{SignatureState, TrustedKey};
use tmc_core::plugins::steps::{RunContext, RunReport};

use crate::state::AppState;

#[tauri::command]
pub fn plugin_list(state: State<'_, AppState>) -> Vec<PluginRecord> {
    state.plugins.list()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPreview {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: Option<String>,
    pub kinds: Vec<String>,
    pub permissions: Vec<String>,
    /// Echoed back to `plugin_approve`, closing the read-then-approve window.
    pub fingerprint: String,
    pub dir: String,

    /// Who vouched for this bundle, if anybody.
    ///
    /// Shown beside the permissions rather than after the fact: who signed a
    /// plugin is part of what the user is deciding about, and finding out later
    /// from a list is finding out too late.
    pub signature: SignatureState,
}

fn kinds_of(manifest: &tmc_core::plugins::manifest::Manifest) -> Vec<String> {
    let mut kinds = Vec::new();

    if manifest.installer.is_some() {
        kinds.push("installer".to_string());
    }
    if manifest.server_query.is_some() {
        kinds.push("serverQuery".to_string());
    }
    if manifest.theme.is_some() {
        kinds.push("theme".to_string());
    }

    kinds
}

/// Read a bundle the user picked, without installing it.
#[tauri::command]
pub fn plugin_inspect(state: State<'_, AppState>, dir: String) -> AppResult<PluginPreview> {
    let path = PathBuf::from(&dir);

    if !path.is_dir() {
        return Err(AppError::invalid("That is not a folder."));
    }

    let (manifest, permissions, signature) = state.plugins.inspect(&path)?;

    Ok(PluginPreview {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        author: manifest.author.clone(),
        description: manifest.description.clone(),
        kinds: kinds_of(&manifest),
        permissions,
        fingerprint: manifest.fingerprint(),
        dir,
        signature,
    })
}

// ------------------------------------------------------------ Trusted keys

/// Every publishing key the user trusts.
#[tauri::command]
pub fn plugin_trusted_keys(state: State<'_, AppState>) -> Vec<TrustedKey> {
    state.plugins.trusted_keys()
}

/// Trust a publishing key.
///
/// Audited at Security level, and deliberately so: this widens what may run on
/// the machine when `requireSignedPlugins` is on, and a change to that must be
/// visible in the log whether or not logging is turned up.
#[tauri::command]
pub fn plugin_trust_key(
    state: State<'_, AppState>,
    id: String,
    label: String,
    public_key: String,
) -> AppResult<TrustedKey> {
    let key = state.plugins.trust_key(&id, &label, &public_key)?;

    audit!(
        state.audit,
        Security,
        Plugin,
        "plugin.key.trust",
        format!("{} ({})", key.label, key.public_key)
    );

    Ok(key)
}

/// Stop trusting one.
#[tauri::command]
pub fn plugin_untrust_key(state: State<'_, AppState>, id: String) -> AppResult<bool> {
    let removed = state.plugins.untrust_key(&id)?;

    if removed {
        audit!(state.audit, Security, Plugin, "plugin.key.untrust", id);
    }

    Ok(removed)
}

#[tauri::command]
pub fn plugin_approve(
    state: State<'_, AppState>,
    dir: String,
    fingerprint: String,
) -> AppResult<PluginRecord> {
    let path = PathBuf::from(&dir);
    let (manifest, _, _) = state.plugins.inspect(&path)?;

    let record = state.plugins.approve(&manifest, &path, &fingerprint)?;

    audit!(
        state.audit,
        Security,
        Plugin,
        "plugin.approve",
        format!(
            "Approved {} v{} with: {}",
            record.name,
            record.version,
            record.permissions.join("; ")
        ),
        plugin = record.id
    );

    Ok(record)
}

#[tauri::command]
pub fn plugin_set_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> AppResult<()> {
    state.plugins.set_enabled(&id, enabled)?;

    if enabled {
        audit!(
            state.audit,
            Security,
            Plugin,
            "plugin.enable",
            "Plugin enabled",
            plugin = id
        );
    } else {
        audit!(
            state.audit,
            Security,
            Plugin,
            "plugin.disable",
            "Plugin disabled",
            plugin = id
        );
    }

    Ok(())
}

#[tauri::command]
pub fn plugin_remove(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.plugins.remove(&id)?;

    audit!(
        state.audit,
        Security,
        Plugin,
        "plugin.remove",
        "Plugin removed",
        plugin = id
    );

    Ok(())
}

#[tauri::command]
pub fn plugin_theme(state: State<'_, AppState>, id: String) -> Option<Theme> {
    state.plugins.theme_tokens(&id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub plugin: String,
    /// `install` or `uninstall`.
    pub action: String,
    /// TMC app id, which selects the `gameDir` root.
    pub app_id: Option<i64>,
    /// Placeholder values for the plan's `{name}` templates.
    #[serde(default)]
    pub context: std::collections::HashMap<String, String>,
}

/// Run an installer plugin's plan.
///
/// The gate order matters and is: registry says the plugin is approved and
/// enabled → the jail is built from the USER's configured directories → the
/// executor runs, auditing each step. There is no path to the executor that
/// skips the first two.
#[tauri::command]
pub async fn plugin_run(state: State<'_, AppState>, request: RunRequest) -> AppResult<RunReport> {
    let manifest = state.plugins.active(&request.plugin)?;

    let installer = manifest
        .installer
        .as_ref()
        .ok_or_else(|| AppError::invalid("That plugin cannot install anything."))?;

    let steps = match request.action.as_str() {
        "install" => &installer.install,
        "uninstall" => &installer.uninstall,
        other => return Err(AppError::invalid(format!("Unknown action '{other}'."))),
    };

    if steps.is_empty() {
        return Err(AppError::invalid(
            "That plugin has no steps for this action.",
        ));
    }

    /*
     * A plugin that declares `apps` may only be pointed at one of them.
     * Otherwise a Minecraft installer could be handed the Rust game directory
     * and would happily write its jars into it — inside the jail, and still
     * wrong.
     */
    if let Some(app_id) = request.app_id {
        if !manifest.apps.is_empty() && !manifest.apps.contains(&app_id) {
            return Err(AppError::jail("This plugin does not handle that game."));
        }
    }

    let settings = state.settings.get();

    let jail =
        tmc_core::plugins::jail_for(&manifest, &state.jail_roots(), &settings, request.app_id)?;

    audit!(
        state.audit,
        Security,
        Install,
        "plugin.run.start",
        format!("{} plan started ({} steps)", request.action, steps.len()),
        plugin = manifest.id
    );

    let executor = tmc_core::plugins::steps::Executor {
        manifest: &manifest,
        jail: &jail,
        http: state.api.raw(),
        audit: &state.audit,
        downloads: Some(&state.downloads),
    };

    let report = executor.run(steps, &RunContext(request.context)).await;

    // Two calls rather than one with a computed level: `audit!` takes the level
    // as an identifier so it can name the enum variant, and an `if` expression
    // does not match that.
    let summary = format!(
        "{} plan {} ({}/{} steps)",
        request.action,
        if report.ok { "completed" } else { "failed" },
        report.steps_run,
        report.steps_total
    );

    if report.ok {
        audit!(
            state.audit,
            Info,
            Install,
            "plugin.run.end",
            summary,
            plugin = manifest.id
        );
    } else {
        audit!(
            state.audit,
            Error,
            Install,
            "plugin.run.end",
            summary,
            plugin = manifest.id
        );
    }

    Ok(report)
}

/// Query a game server through a plugin's protocol description.
///
/// The escape hatch for games this build has no native protocol for. Same
/// public-address guard as everything else in `tmc_core::net`.
#[tauri::command]
pub async fn plugin_query_server(
    state: State<'_, AppState>,
    plugin: String,
    host: String,
    port: u16,
    query_port: Option<u16>,
) -> AppResult<serde_json::Map<String, Value>> {
    let manifest = state.plugins.active(&plugin)?;

    let spec = manifest
        .server_query
        .as_ref()
        .ok_or_else(|| AppError::invalid("That plugin cannot query servers."))?;

    tmc_core::plugins::query::run(spec, &host, port, query_port).await
}
