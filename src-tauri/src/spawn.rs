//! Starting a game from a [`LaunchPlan`], and keeping track of it afterwards.
//!
//! One place, shared by the install launcher and the sandbox launcher, because
//! there are two ways to start a game and they must not drift:
//!
//!   * **Ordinarily** — an argv vector handed to the session registry, which
//!     spawns it and watches it, or the OS opener for a `steam://`-style
//!     hand-off.
//!   * **With a virtual filesystem** — `tmc_usvfs::inject`, which creates the
//!     process suspended, loads the hook DLL into it and only then lets it run.
//!
//! Which one is used is decided by the PLAN, not by the caller, so a sandbox
//! deployed virtually cannot be launched into a stock game folder by going in
//! through the wrong command.
//!
//! Nothing here interprets a string. The plan's arguments are already an argv
//! vector, checked for control characters where they were built; there is no
//! shell in either path, so `;`, backticks and `$(…)` are inert bytes to the
//! child.
//!
//! WHY THIS RETURNS A SESSION
//! -------------------------
//! It used to `Command::spawn` and drop the handle, with a comment explaining
//! that the game outlives the launcher. The first half of that is right and the
//! conclusion was not: dropping the handle also dropped every fact about the
//! launch, which is why the account was told `playedSeconds: 0` on every start,
//! why the Library could not say whether a game was already open, and why a
//! game that died in two seconds left nothing to read.
//!
//! [`tmc_core::session::Sessions`] keeps the handle on a supervisor thread
//! instead. The game still outlives the launcher — closing the app does not
//! kill it, because nothing waits on the child from the UI's thread and the
//! child is not in this process's job object.
//!
//! A URI hand-off gets a session too, and it is recorded as a
//! [`SessionKind::Handoff`] with no duration. That is the honest record: Steam
//! started the game, we never saw a process, and a launcher that invents a play
//! session out of a URI it fired is a launcher whose statistics are fiction.

use tauri::AppHandle;

use tmc_core::error::{AppError, AppResult};
use tmc_core::launch::LaunchPlan;
use tmc_core::session::{Session, SessionSpec};

use crate::state::AppState;

/// Start whatever the plan describes, and open a session for it.
pub fn run(
    app: &AppHandle,
    state: &AppState,
    plan: &LaunchPlan,
    spec: SessionSpec,
) -> AppResult<Session> {
    if let Some(uri) = &plan.uri {
        /*
         * Handed to the OS opener, which is what resolves a `steam://` link to
         * the installed client. The scheme was checked at load time
         * (`is_allowed_launch_uri`) and the value carries no control
         * characters, so there is nothing here that could become a second
         * argument.
         */
        use tauri_plugin_opener::OpenerExt;

        app.opener()
            .open_url(uri.clone(), None::<&str>)
            .map_err(|e| AppError::internal(format!("could not open {uri}: {e}")))?;

        return Ok(state.sessions.record_handoff(spec));
    }

    let Some(program) = &plan.program else {
        return Err(AppError::invalid(
            "That launch rule names nothing to start.",
        ));
    };

    if let Some(vfs) = &plan.vfs {
        launch_with_vfs(program, plan, vfs)?;

        /*
         * The injector creates the process itself, so there is no `Child` to
         * hand the registry — it returns once the suspended process has been
         * resumed. Recorded as a hand-off for that reason and not because
         * nothing of ours started it: what is missing is the HANDLE, and
         * without one there is no exit to observe and no duration to measure.
         *
         * Making this measurable means teaching `inject::launch` to return its
         * process handle, which is a change inside the one part of the tree
         * that has never been run against a game. It is listed as not built.
         */
        return Ok(state.sessions.record_handoff(spec));
    }

    let mut command = std::process::Command::new(program);

    // An argv VECTOR. There is no shell anywhere in this, so quoting, `;`,
    // backticks and `$( )` are inert bytes to the child.
    command.args(&plan.args);

    if let Some(cwd) = &plan.cwd {
        command.current_dir(cwd);
    }

    for (key, value) in &plan.env {
        command.env(key, value);
    }

    state.sessions.spawn(command, spec)
}

/// Start the game with the sandbox's virtual filesystem loaded into it.
///
/// Only compiled on Windows and only with `usvfs-hooks`; every other build
/// takes the refusal below. That is not defensive coding — it is the same gate
/// `deploy` applies, restated at the one other place it could be bypassed. A
/// build that can deploy virtually can launch virtually and vice versa.
#[cfg(all(windows, feature = "usvfs-hooks"))]
fn launch_with_vfs(
    program: &str,
    plan: &LaunchPlan,
    vfs: &tmc_core::launch::VfsHandoff,
) -> AppResult<()> {
    use std::path::Path;

    /*
     * The DLL sits beside the executable, as a bundled resource. Resolved from
     * the running binary rather than from a setting or the plan: this path is
     * handed to `LoadLibraryW` INSIDE the game's process, so anything that
     * could influence it is arbitrary code execution with the game's
     * privileges.
     */
    let dll = std::env::current_exe()
        .map_err(|e| AppError::internal(format!("could not locate the app's own folder: {e}")))?
        .parent()
        .map(|dir| dir.join(DLL_NAME))
        .ok_or_else(|| AppError::internal("the app's own folder has no parent"))?;

    if !dll.is_file() {
        return Err(AppError::invalid(
            "This sandbox deploys virtually, but the app's virtual-filesystem component is \
             missing from the installation. Reinstall the app, or switch the sandbox to hard \
             links.",
        ));
    }

    // The environment the DLL reads on attach, layered over the plan's own.
    let mut env: Vec<(String, String)> = plan
        .env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    env.push((tmc_usvfs::ENV_BLOB.to_string(), vfs.blob.clone()));
    env.push((tmc_usvfs::ENV_ROOT.to_string(), vfs.root.clone()));

    tmc_usvfs::inject::launch(&tmc_usvfs::inject::LaunchSpec {
        program: Path::new(program),
        args: &plan.args,
        working_dir: plan.cwd.as_deref().map(Path::new),
        env: &env,
        dll: &dll,
    })
    .map_err(|e| AppError::internal(format!("could not start the game with its mods: {e}")))?;

    Ok(())
}

/// The bundled hook DLL's file name.
#[cfg(all(windows, feature = "usvfs-hooks"))]
const DLL_NAME: &str = "tmc_usvfs.dll";

#[cfg(not(all(windows, feature = "usvfs-hooks")))]
fn launch_with_vfs(
    _program: &str,
    _plan: &LaunchPlan,
    _vfs: &tmc_core::launch::VfsHandoff,
) -> AppResult<()> {
    Err(AppError::invalid(
        tmc_core::deploy::usvfs_unavailable()
            .unwrap_or("Virtual-filesystem deployment is not available here."),
    ))
}
