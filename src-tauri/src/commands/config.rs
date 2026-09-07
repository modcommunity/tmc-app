//! **A game's own settings files**, opened from inside the app.
//!
//! The feature every studied mod manager has, and the one this app was missing:
//! a loader writes `BepInEx/config/com.author.mod.cfg` the first time the game
//! runs, one value in it is wrong, and fixing it currently means leaving the
//! manager and finding the folder.
//!
//! WHAT THIS DOES AND DOES NOT WIDEN
//! --------------------------------
//! It is worth being exact, because it is easy to overstate in both directions.
//!
//! The app PROCESS has always had the user's filesystem permissions and must —
//! `deploy::link` writes into game folders, the plugin executor unpacks
//! archives into them. Nothing here changes that and nothing here needed to.
//!
//! What these three commands add is the ability for the WEBVIEW to name a file
//! to be read or written, which is the rule at the top of
//! [`crate::commands`] and the reason a mod description rendered next to
//! `invoke` is a nuisance rather than a problem. So the surface is bounded the
//! way every other privileged surface here is:
//!
//!   * **The GAME decides which files exist.** `plugins/app/<slug>/config.json`
//!     declares them, exactly as `manage_mod.json` declares where mods go. A
//!     game with no such file has no config editor, which is honest — sweeping
//!     the game folder for `*.cfg` would offer somebody their save file.
//!   * **Two bounds, not one.** The declared locations compile to jail grants,
//!     and [`tmc_core::plugins::config`] re-runs the spec's own matching rules
//!     on every read and write. The jail alone is not enough: a location naming
//!     the game's root, which `{ path: "", files: ["options.txt"] }`
//!     legitimately does, would otherwise let `saves/world.dat` through.
//!   * **A switch.** `allowConfigEditing` removes these commands. It buys
//!     exactly that and is documented as buying exactly that.
//!
//! WHY IT IS KEYED ON A SANDBOX
//! ---------------------------
//! Because "which game folder" is a question only a sandbox answers. Two
//! sandboxes for one game routinely deploy into different directories, and a
//! config editor pointed at the wrong one edits settings the running game will
//! never read — which presents as the editor not working.

use serde::Serialize;
use tauri::State;

use tmc_core::error::{AppError, AppResult};
use tmc_core::plugins::config::{ConfigFile, WriteOutcome};
use tmc_core::plugins::manifest::FsRoot;

use crate::state::AppState;

/// Everything one call needs: the game's rules, and a jail over its folders.
struct Context {
    spec: tmc_core::plugins::apps::ConfigSpec,
    jail: tmc_core::plugins::jail::Jail,
}

/// Resolve a sandbox to its game's config rules and a jail over them.
///
/// The setting is checked HERE rather than in each command, so a command added
/// beside these three cannot forget it.
fn context(state: &AppState, sandbox_id: i64) -> AppResult<Context> {
    if !state.settings.get().allow_config_editing {
        return Err(AppError::invalid(
            "Editing game settings from the app is turned off in Settings → App.",
        ));
    }

    let sandbox = state
        .library
        .sandbox_get(sandbox_id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let slug = sandbox
        .app_slug
        .clone()
        .ok_or_else(|| AppError::invalid("That sandbox is not linked to a game's rules."))?;

    let plugins = state.app_plugins();

    let file = plugins.config_file(&slug).ok_or_else(|| {
        AppError::invalid("This game does not describe where it keeps its settings.")
    })?;

    let spec = file
        .config
        .clone()
        .ok_or_else(|| AppError::internal("a config rule with no config"))?;

    /*
     * The SANDBOX's folder, not the app's.
     *
     * `target_dir` is the same helper every screen showing a sandbox uses, so
     * the editor opens the files the game about to be launched from this
     * sandbox will actually read. Overriding the settings map is how the rest
     * of the app does this too — see `AppState::launch_plan`.
     */
    let mut settings = state.settings.get();

    let dir = tmc_core::library::deploy::target_dir(&sandbox, &settings).map_err(|_| {
        AppError::invalid(
            "No game folder is set for this sandbox, so there is nothing to read settings from.",
        )
    })?;

    settings.game_dirs.insert(
        sandbox.app_id.to_string(),
        dir.to_string_lossy().into_owned(),
    );

    let jail = tmc_core::plugins::jail_for(
        &file.as_manifest(),
        &state.jail_roots(),
        &settings,
        Some(sandbox.app_id),
    )?;

    Ok(Context { spec, jail })
}

/// Whether this sandbox's game offers a config editor at all.
///
/// Its own command so the UI can leave the tab out entirely rather than showing
/// one that opens onto an error — a game with no `config.json` is the common
/// case, not a failure.
#[tauri::command]
pub fn config_available(state: State<'_, AppState>, sandbox_id: i64) -> bool {
    context(&state, sandbox_id).is_ok()
}

/// Every settings file this game offers, for this sandbox.
#[tauri::command]
pub fn config_list(state: State<'_, AppState>, sandbox_id: i64) -> AppResult<Vec<ConfigFile>> {
    let ctx = context(&state, sandbox_id)?;

    Ok(tmc_core::plugins::config::list(&ctx.spec, &ctx.jail))
}

#[tauri::command]
pub fn config_read(
    state: State<'_, AppState>,
    sandbox_id: i64,
    root: FsRoot,
    path: String,
) -> AppResult<String> {
    let ctx = context(&state, sandbox_id)?;

    tmc_core::plugins::config::read(&ctx.jail, &ctx.spec, root, &path)
}

/// What a save did, plus whether the sandbox now needs redeploying.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveReport {
    #[serde(flatten)]
    pub outcome: WriteOutcome,
}

#[tauri::command]
pub fn config_write(
    state: State<'_, AppState>,
    sandbox_id: i64,
    root: FsRoot,
    path: String,
    contents: String,
) -> AppResult<SaveReport> {
    let ctx = context(&state, sandbox_id)?;

    let outcome = tmc_core::plugins::config::write(
        &ctx.jail,
        &ctx.spec,
        root,
        &path,
        &contents,
        &state.paths.backup_dir(),
    )?;

    /*
     * Audited at Security level, alongside the plugin steps and the jail
     * refusals. This writes into a folder the user pointed the app at, at the
     * webview's request — which is precisely the class of thing the activity
     * log exists to record, and it must not be hideable by turning verbose
     * logging off.
     */
    tmc_core::audit!(
        state.audit,
        Security,
        Install,
        "config.write",
        format!("{path} ({} bytes)", outcome.bytes)
    );

    Ok(SaveReport { outcome })
}
