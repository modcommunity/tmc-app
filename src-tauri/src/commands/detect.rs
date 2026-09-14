//! Finding installed games, and offering what was found.
//!
//! **Detection suggests; the user applies.** `detect_games` is read-only and
//! returns candidates. `detect_apply` is what actually points a game directory
//! somewhere, and it runs the same [`tmc_core::anchor::validate_root`] a
//! hand-typed path does — a folder is not more trustworthy for having been
//! found automatically, and a game directory is a jail anchor.
//!
//! The two are separate commands rather than one for exactly that reason: a
//! scan that configured as it went would be a scan that could not be shown to
//! anybody first.
//!
//! TWO WAYS TO LOOK
//! ---------------
//! [`detect_games`] reads what the LAUNCHERS wrote down — Steam's manifests,
//! Epic's `.item` files, Galaxy's database. It is fast, exact and covers most
//! machines, and it is what runs when the screen opens.
//!
//! [`detect_scan`] walks folders **the user ticked**, for everything the
//! launchers do not know about: a game copied from another PC, a dedicated
//! server unpacked by hand, a drive that was moved. It is slower, it is bounded
//! on four axes, and it never runs on its own — see [`tmc_core::detect::walk`]
//! for why the roots are a required argument with no default.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use tmc_core::detect::walk::{WalkLimits, WalkReport};
use tmc_core::detect::{DetectReport, DetectedGame};
use tmc_core::error::{AppError, AppResult};
use tmc_core::settings::AppSettings;

use crate::state::AppState;

/// A candidate, plus what applying it would mean.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    #[serde(flatten)]
    pub game: DetectedGame,
    /// Is this folder already configured for its game?
    pub already_set: bool,
    /// Would applying it replace a different folder?
    pub replaces: Option<String>,
}

/// Everything this machine has installed that the app can recognise.
///
/// The scan touches the filesystem, so it runs off the UI thread. It is not
/// cached: somebody opens this screen because they just installed something.
#[tauri::command]
pub async fn detect_games(app: AppHandle, state: State<'_, AppState>) -> AppResult<Vec<Candidate>> {
    let roots = state.detect_roots(&app);
    let plugins = state.app_plugins();
    let settings = state.settings.get();

    let report: DetectReport =
        tauri::async_runtime::spawn_blocking(move || tmc_core::detect::scan(&roots, &plugins))
            .await
            .map_err(|e| AppError::internal(format!("detect: {e}")))?;

    /*
     * Learned BEFORE annotating, so `alreadySet` and `replaces` are right on
     * the first render. Without it a scan of a fresh machine shows every game
     * as unconfigured, including the ones that are — the lookup that decides it
     * had no id to compare against.
     */
    let slugs: Vec<String> = report.games.iter().filter_map(|g| g.slug.clone()).collect();

    learn_app_ids(&state, &slugs).await;

    Ok(report
        .games
        .into_iter()
        .map(|game| annotate(game, &settings, &state))
        .collect())
}

fn annotate(game: DetectedGame, settings: &AppSettings, state: &AppState) -> Candidate {
    let configured = game
        .slug
        .as_deref()
        .and_then(|slug| app_id_for(slug, state))
        .and_then(|id| settings.game_dirs.get(&id.to_string()).cloned());

    let already_set = configured
        .as_deref()
        .is_some_and(|current| same_folder(current, &game.path));

    Candidate {
        already_set,
        replaces: configured.filter(|_| !already_set),
        game,
    }
}

/// Point a game's directory at a detected folder.
///
/// Takes the SLUG and the path, and resolves the app id itself — the frontend
/// naming an app id and a path together is how a mismatched pair ends up
/// pointing one game's installer at another game's folder.
#[tauri::command]
pub fn detect_apply(state: State<'_, AppState>, slug: String, path: String) -> AppResult<()> {
    let app_id = app_id_for(&slug, &state).ok_or_else(|| {
        /*
         * Name the half that is actually missing.
         *
         * Reaching here means the app HAS a rule for this game — that is the
         * only reason the scan could name it — and the CATALOGUE has no row to
         * key a game directory on. The old wording said the app did not know
         * the game, which sent everybody to look at `plugins/app/` where the
         * rule was sitting perfectly correctly, and never at the site.
         */
        AppError::invalid(format!(
            "This build has rules for {slug}, but the site's catalogue has no \
             app with that name — so there is nothing to attach the folder to. \
             It usually means the game has not been added to the site yet, or \
             that the rule's folder name and the game's URL name differ."
        ))
    })?;

    let (_, stored) =
        state
            .settings
            .set_game_dir(&app_id.to_string(), Some(&path), &protected(&state))?;

    tmc_core::audit!(
        state.audit,
        Security,
        App,
        "detect.apply",
        format!("{slug} → {}", stored.clone().unwrap_or_default())
    );

    Ok(())
}

/// Which TMC app id a slug belongs to on this device.
///
/// A slug is the plugin folder's name and an id is the API's, and everything
/// the device stores — a game directory, a sandbox — is keyed by the id. So
/// something has to translate, and the local sources only can once the machine
/// already has a sandbox or a subscription for that game.
///
/// **That was the whole story until the folder scan existed, and it failed for
/// the exact case the scan is for.** A fresh install with an empty library
/// found a dozen games and could apply none of them, with an error saying the
/// app did not know the game — which it did; it only did not know its id.
///
/// The catalogue is the third source and the authoritative one. It is cached
/// (see [`AppState::cached_app_id`]) because a batch apply asks once per
/// candidate, and it is filled by [`learn_app_ids`] before a batch runs.
fn app_id_for(slug: &str, state: &AppState) -> Option<i64> {
    if let Ok(sandboxes) = state.library.sandbox_list(None) {
        if let Some(found) = sandboxes
            .iter()
            .find(|s| s.app_slug.as_deref() == Some(slug))
        {
            return Some(found.app_id);
        }
    }

    if let Ok(rows) = state.library.list() {
        if let Some(id) = rows
            .into_iter()
            .find(|e| e.app_slug.as_deref() == Some(slug))
            .and_then(|e| e.app_id)
        {
            return Some(id);
        }
    }

    state.cached_app_id(slug)
}

/// Ask the catalogue for the ids behind a set of slugs, and cache them.
///
/// Best effort throughout: a device that is offline still has whatever it
/// learned last time, and a slug the catalogue does not know is a game the app
/// ships a rule for and the site does not list — which is a real state during
/// development and must not fail the whole batch.
async fn learn_app_ids(state: &AppState, slugs: &[String]) {
    let wanted: Vec<String> = slugs
        .iter()
        .filter(|slug| state.cached_app_id(slug).is_none())
        .cloned()
        .collect();

    if wanted.is_empty() {
        return;
    }

    /*
     * Encoded here rather than through `commands::api::api_get`, which is the
     * webview's door and takes its path from a caller. A slug is a plugin
     * folder's name — `[a-z0-9-]` by the time it reaches this — but it is still
     * interpolated into a URL, so it goes through the same percent-encoding
     * every other query value does.
     */
    let encoded = wanted
        .iter()
        .take(100)
        .map(|slug| format!("slugs={}", urlencode(slug)))
        .chain(std::iter::once("limit=100".to_string()))
        .collect::<Vec<_>>()
        .join("&");

    let Ok(answer) = state
        .api
        .request(
            tmc_core::api::Method::GET,
            &format!("/apps?{encoded}"),
            None,
            false,
        )
        .await
    else {
        return;
    };

    let Some(apps) = answer.get("apps").and_then(serde_json::Value::as_array) else {
        return;
    };

    state.remember_app_ids(apps.iter().filter_map(|app| {
        let id = app.get("id")?.as_i64()?;
        let slug = app.get("slug")?.as_str()?.to_string();

        Some((slug, id))
    }));
}

/// Are these the same directory, allowing for a symlink or a trailing slash?
fn same_folder(a: &str, b: &str) -> bool {
    tmc_core::canon::canonicalize_or_keep(a) == tmc_core::canon::canonicalize_or_keep(b)
}

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

// --------------------------------------------------------- Scanning by hand

/// Progress from a running scan.
///
/// One event per directory would be tens of thousands of IPC messages for a
/// drive, so they are throttled to [`PROGRESS_EVERY`]. The UI shows the path
/// because a scan of a whole drive takes long enough that a spinner with
/// nothing under it reads as a hang — and because seeing it sitting in a
/// backup folder is how somebody learns to tick a narrower root.
const EVENT_PROGRESS: &str = "tmc://scan-progress";

/// Directories between progress events.
const PROGRESS_EVERY: usize = 200;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScanProgress {
    dirs: usize,
    /// Where the walk currently is. Display only.
    path: String,
}

/// What the frontend asks for.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRequest {
    /// The folders the user ticked. Required, and there is no default — see
    /// [`tmc_core::detect::walk`] for why a scan never chooses its own roots.
    pub roots: Vec<String>,
    #[serde(default)]
    pub limits: Option<WalkLimits>,
}

/// A scan result, with the same annotation a launcher-found candidate gets.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub games: Vec<Candidate>,
    pub dirs_visited: usize,
    pub elapsed_ms: u64,
    pub stop: tmc_core::detect::walk::WalkStop,
    pub unreadable: Vec<(String, String)>,
}

/// Walk the folders the user chose.
///
/// **Cancellable and single-flight.** A second scan while one is running is
/// refused rather than queued: two walks share one directory budget in the
/// user's head and neither would finish in the time they expected. The flag
/// lives in the state so `detect_scan_cancel` can reach it.
#[tauri::command]
pub async fn detect_scan(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ScanRequest,
) -> AppResult<ScanReport> {
    if request.roots.is_empty() {
        return Err(AppError::invalid(
            "Choose at least one folder or drive to scan.",
        ));
    }

    if request.roots.len() > WalkLimits::MAX_ROOTS {
        return Err(AppError::invalid(format!(
            "That is more than {} folders. Scan them in batches.",
            WalkLimits::MAX_ROOTS
        )));
    }

    if !state.scan_begin() {
        return Err(AppError::invalid(
            "A scan is already running. Wait for it to finish, or cancel it.",
        ));
    }

    /*
     * Cleared AFTER the slot is claimed, not before. A cancel that arrives
     * between the two would otherwise be swallowed and the new scan would run
     * to completion having been told to stop.
     */
    let cancel = state.scan_cancel();

    cancel.store(false, Ordering::SeqCst);

    let roots: Vec<PathBuf> = request.roots.iter().map(PathBuf::from).collect();
    let limits = request.limits.unwrap_or_default();
    let plugins = state.app_plugins();

    tmc_core::audit!(
        state.audit,
        Info,
        App,
        "detect.scan",
        format!("scanning {} folder(s)", roots.len())
    );

    let emitter = app.clone();
    let flag = Arc::clone(&cancel);

    /*
     * `spawn_blocking`, because the walk is `read_dir` in a loop and would hold
     * a runtime worker for up to the whole time budget. The progress callback
     * runs on that thread and emits directly — Tauri's emitter is `Send` and an
     * event is a fire-and-forget write to the webview.
     */
    let walked: AppResult<WalkReport> = tauri::async_runtime::spawn_blocking(move || {
        let mut last = 0usize;

        let mut progress = |path: &std::path::Path, dirs: usize| -> bool {
            if dirs >= last + PROGRESS_EVERY {
                last = dirs;

                let _ = emitter.emit(
                    EVENT_PROGRESS,
                    ScanProgress {
                        dirs,
                        path: path.display().to_string(),
                    },
                );
            }

            !flag.load(Ordering::SeqCst)
        };

        tmc_core::detect::walk::walk(&roots, &plugins, &limits, &mut progress)
    })
    .await
    .map_err(|e| AppError::internal(format!("scan: {e}")));

    // Cleared on every path, including the error one — a flag left set makes
    // the next scan refuse to start with a message about a scan that is gone.
    state.scan_end();
    cancel.store(false, Ordering::SeqCst);

    let report = walked?;

    let settings = state.settings.get();

    let slugs: Vec<String> = report.games.iter().filter_map(|g| g.slug.clone()).collect();

    learn_app_ids(&state, &slugs).await;

    Ok(ScanReport {
        games: report
            .games
            .into_iter()
            .map(|game| annotate(game, &settings, &state))
            .collect(),
        dirs_visited: report.dirs_visited,
        elapsed_ms: report.elapsed_ms,
        stop: report.stop,
        unreadable: report.unreadable,
    })
}

/// Stop a running scan.
///
/// The walk checks the flag between directories, so this takes effect within
/// one `read_dir` rather than immediately — which on a dead network share can
/// still be several seconds. The UI says "stopping" rather than closing itself,
/// because a dialog that vanishes while the thread is still walking is one that
/// will refuse the next scan for reasons nobody can see.
#[tauri::command]
pub fn detect_scan_cancel(state: State<'_, AppState>) {
    state.scan_cancel().store(true, Ordering::SeqCst);
}

/// Whether a scan is running, so a reopened screen shows the right state.
#[tauri::command]
pub fn detect_scan_running(state: State<'_, AppState>) -> bool {
    state.scan_running()
}

/// One result of applying a batch of candidates.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOutcome {
    pub slug: String,
    pub path: String,
    pub ok: bool,
    /// Why it was refused. Present exactly when `ok` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Point several games' directories at what a scan found.
///
/// **Every one goes through [`detect_apply`]'s own validation**, one at a time,
/// and a failure does not stop the rest. That is the difference between this
/// and a loop in the frontend: a batch where one folder is refused must still
/// apply the other eleven, and the user needs to be told which one and why
/// rather than being shown a single error for the whole operation.
///
/// It is the "automatically integrate what it finds" half of the scan button,
/// and it is still a separate call from the scan itself — the user sees the
/// list and confirms it. A scan that configured as it went could not be shown
/// to anybody first, which is the rule at the top of this module.
#[tauri::command]
pub fn detect_apply_many(
    state: State<'_, AppState>,
    games: Vec<(String, String)>,
) -> Vec<ApplyOutcome> {
    let mut out = Vec::with_capacity(games.len());

    for (slug, path) in games {
        match detect_apply(state.clone(), slug.clone(), path.clone()) {
            Ok(()) => out.push(ApplyOutcome {
                slug,
                path,
                ok: true,
                error: None,
            }),
            Err(err) => out.push(ApplyOutcome {
                slug,
                path,
                ok: false,
                error: Some(err.to_string()),
            }),
        }
    }

    out
}

/// Percent-encode everything outside the unreserved set.
///
/// A second copy of `commands::api`'s helper rather than a shared one, and that
/// is a deliberate two lines: making it public would put a URL-building helper
/// on the module the webview talks to, next to the command that takes a path
/// from it.
fn urlencode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());

    for byte in raw.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }

    out
}
