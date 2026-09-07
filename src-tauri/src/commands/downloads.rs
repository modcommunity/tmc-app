//! The download queue's IPC surface.
//!
//! **No command here takes a destination.** A download's target comes from the
//! thing that queued it — a sandbox's staging folder, a plugin step's jailed
//! path — and everything the webview can do is act on a queue row it can
//! already see: pause it, resume it, cancel it, reprioritise it, throttle it.
//!
//! That is the whole reason there is no `download_start(url, path)`. Such a
//! command would be an arbitrary-write primitive with a progress bar attached,
//! reachable from a rendered mod description.
//!
//! Progress reaches the UI as an EVENT rather than a poll. A queue of four
//! hundred files ticking twice a second is eight hundred renders a second if
//! the frontend asks; as an event stream it is one message per changed row, and
//! the webview coalesces them itself.

use std::sync::atomic::{AtomicI64, Ordering};

use serde::Serialize;
use tauri::{Emitter, Manager, State};

use tmc_core::audit;
use tmc_core::download::{DownloadEvent, DownloadState, Status};
use tmc_core::error::{AppError, AppResult};

use crate::state::AppState;

/// How often the queue is reported to the website, in milliseconds.
///
/// A heartbeat, not an action. Five seconds is faster than the numbers on the
/// website's page move and slow enough that a busy queue is not a request per
/// tick — a running download changes twice a second and eight of them would be
/// sixteen POSTs a second otherwise.
const REPORT_EVERY_MS: i64 = 5_000;

/// Rows sent with a report. Matches `MAX_REPORTED_DOWNLOADS` in the contract.
const MAX_REPORTED: usize = 25;

/// When the queue was last reported, as epoch millis.
///
/// A process-global rather than state on `AppState`: the bridge is spawned once
/// and is the only writer, and threading a clock through it would be ceremony
/// for one integer.
static LAST_REPORT_MS: AtomicI64 = AtomicI64::new(0);

/// The event name the frontend listens on.
pub const EVENT: &str = "tmc://download";

/// The queue, plus the two totals every screen showing it wants.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub downloads: Vec<DownloadState>,
    pub active: usize,
    /// Combined bytes per second across everything running.
    pub speed_bps: u64,
    /// The global ceiling, or `None` for unlimited.
    pub limit_bps: Option<u64>,
}

#[tauri::command]
pub async fn download_list(state: State<'_, AppState>) -> AppResult<QueueSnapshot> {
    let downloads = state.downloads.list().await;

    Ok(QueueSnapshot {
        active: downloads.iter().filter(|d| d.status.is_active()).count(),
        speed_bps: downloads
            .iter()
            .filter(|d| d.status == Status::Running)
            .map(|d| d.speed_bps)
            .sum(),
        limit_bps: state.downloads.global_limit().await,
        downloads,
    })
}

#[tauri::command]
pub async fn download_pause(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.downloads.pause(&id).await
}

#[tauri::command]
pub async fn download_resume(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.downloads.resume(&id).await
}

#[tauri::command]
pub async fn download_cancel(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.downloads.cancel(&id).await?;

    // The persisted row goes too — a cancelled download is not something to
    // restore on the next launch.
    let _ = state.library.download_delete(&id);

    Ok(())
}

/// Move one up or down the queue. Higher runs first.
#[tauri::command]
pub async fn download_set_priority(
    state: State<'_, AppState>,
    id: String,
    priority: i32,
) -> AppResult<()> {
    state
        .downloads
        .set_priority(&id, priority.clamp(-100, 100))
        .await
}

/// This download's own ceiling, in bytes per second. `None` or `0` removes it.
#[tauri::command]
pub async fn download_set_limit(
    state: State<'_, AppState>,
    id: String,
    bps: Option<u64>,
) -> AppResult<()> {
    state.downloads.set_limit(&id, bps).await
}

/// The ceiling across everything, in bytes per second.
///
/// Persisted in app settings, so it survives a restart — a bandwidth limit
/// somebody set because their connection is shared is not a per-session
/// preference.
#[tauri::command]
pub async fn download_set_global_limit(
    state: State<'_, AppState>,
    bps: Option<u64>,
) -> AppResult<()> {
    state.downloads.set_global_limit(bps).await;

    state.settings.patch(serde_json::json!({
        "downloadLimitBps": bps.unwrap_or(0),
    }))?;

    Ok(())
}

/// How many transfers may run at once.
#[tauri::command]
pub async fn download_set_concurrency(state: State<'_, AppState>, n: usize) -> AppResult<()> {
    state.downloads.set_concurrency(n).await;

    state.settings.patch(serde_json::json!({
        "downloadConcurrency": n.clamp(1, tmc_core::download::MAX_CONCURRENT),
    }))?;

    Ok(())
}

#[tauri::command]
pub async fn download_clear_finished(state: State<'_, AppState>) -> AppResult<usize> {
    let cleared = state.downloads.clear_finished().await;

    let _ = state.library.download_clear_finished();

    Ok(cleared)
}

/// Forward queue events to the webview, and persist every change.
///
/// Spawned once at startup. Two jobs in one loop because both are driven by the
/// same event and doing them separately would mean two subscribers, two
/// wakeups, and a window where the UI shows a state the database does not have.
pub fn spawn_bridge(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let (mut events, library) = {
            let state = app.state::<AppState>();

            (
                state.downloads.subscribe(),
                std::sync::Arc::clone(&state.library),
            )
        };

        loop {
            match events.recv().await {
                Ok(DownloadEvent::Progress { download }) => {
                    /*
                     * Persisted only on a state CHANGE, not on every progress
                     * tick. A tick is twice a second per download; writing all
                     * of them would be hundreds of transactions a second for a
                     * number that is reconstructible from the `.part` file.
                     */
                    if !matches!(download.status, Status::Running) {
                        let _ = library.download_save(&download);
                    }

                    let _ = app.emit(EVENT, &*download);

                    /*
                     * A terminal transition is reported immediately; everything
                     * else waits for the heartbeat. "It finished" is the one
                     * update somebody watching from their phone is actually
                     * waiting for, and making them wait five more seconds for
                     * it is the difference between the page feeling live and
                     * feeling stale.
                     */
                    report_soon(&app, download.status.is_finished()).await;
                }
                Ok(DownloadEvent::Idle) => {
                    let _ = app.emit(EVENT, serde_json::json!({ "type": "idle" }));

                    report_soon(&app, true).await;
                }
                // Only when the manager is gone, which is process shutdown.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                // A burst outran this subscriber. The next event carries the
                // current state, so there is nothing to recover.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });
}

/// Send the queue to the website, if it is time.
///
/// `now` forces it past the heartbeat interval — used for a terminal
/// transition, which is the update somebody watching from another device is
/// actually waiting for.
///
/// Every failure is swallowed. This is a courtesy to a screen on another
/// device; a signed-out app, an offline laptop or a 500 from the API must not
/// produce an error on the machine that is downloading perfectly well.
async fn report_soon(app: &tauri::AppHandle, now: bool) {
    let millis = tmc_core::logging::epoch_millis();

    if !now {
        let last = LAST_REPORT_MS.load(Ordering::Relaxed);

        if millis - last < REPORT_EVERY_MS {
            return;
        }
    }

    LAST_REPORT_MS.store(millis, Ordering::Relaxed);

    let state = app.state::<AppState>();

    /*
     * Nothing to report to. Checked on the REFRESH token as well as the access
     * one: the access token lasts an hour and a laptop that has been asleep
     * has none, but the API client will mint one on the first request — so
     * skipping on a missing access token alone would stop reporting for
     * exactly the session that has been running longest.
     */
    if state.auth.access_token().is_none() && !state.auth.has_refresh() {
        return;
    }

    let downloads = state.downloads.list().await;

    let active = downloads
        .iter()
        .filter(|d| d.status == Status::Running)
        .count();
    let queued = downloads
        .iter()
        .filter(|d| d.status == Status::Queued)
        .count();
    let paused = downloads
        .iter()
        .filter(|d| d.status == Status::Paused)
        .count();
    let failed = downloads
        .iter()
        .filter(|d| d.status == Status::Failed)
        .count();
    let finished = downloads
        .iter()
        .filter(|d| d.status == Status::Done)
        .count();

    /*
     * An empty queue DELETES the row rather than reporting zeroes. A device
     * with nothing to say should disappear from the website's list instead of
     * sitting there at zero forever, which reads as a machine that is stuck.
     */
    if active + queued + paused + failed == 0 {
        let _ = state
            .api
            .request(tmc_core::api::Method::DELETE, "/downloads", None, true)
            .await;

        return;
    }

    let remaining: u64 = downloads
        .iter()
        .filter(|d| d.status.is_active() || d.status == Status::Paused)
        .map(|d| d.total.unwrap_or(0).saturating_sub(d.done))
        .sum();

    /*
     * The rows worth SHOWING, not the whole queue. Running first, then paused
     * and failed — a modpack is four hundred items and the website's page
     * renders the summary counters above for the rest.
     */
    let mut interesting: Vec<&DownloadState> = downloads
        .iter()
        .filter(|d| matches!(d.status, Status::Running | Status::Paused | Status::Failed))
        .collect();

    interesting.sort_by_key(|d| match d.status {
        Status::Running => 0,
        Status::Failed => 1,
        _ => 2,
    });

    let items: Vec<serde_json::Value> = interesting
        .into_iter()
        .take(MAX_REPORTED)
        .map(|d| {
            serde_json::json!({
                "id": d.id,
                "label": d.label,
                "status": d.status.as_str(),
                "done": d.done,
                "total": d.total,
                "speedBps": d.speed_bps,
            })
        })
        .collect();

    let body = serde_json::json!({
        "active": active,
        "queued": queued,
        "paused": paused,
        "failed": failed,
        "finished": finished,
        "speedBps": downloads
            .iter()
            .filter(|d| d.status == Status::Running)
            .map(|d| d.speed_bps)
            .sum::<u64>(),
        "remainingBytes": remaining,
        "items": items,
    });

    if let Err(err) = state
        .api
        .request(tmc_core::api::Method::POST, "/downloads", Some(body), true)
        .await
    {
        tracing::debug!("could not report downloads: {}", err.detail());
    }
}

/// Restore the queue from the last session and apply the saved limits.
///
/// Called once at startup. Paused rows come back paused; everything else that
/// was unfinished goes back in the queue and starts on its own.
pub async fn restore(state: &AppState) {
    let settings = state.settings.get();

    state
        .downloads
        .set_global_limit(Some(settings.download_limit_bps).filter(|b| *b > 0))
        .await;

    state
        .downloads
        .set_concurrency(settings.download_concurrency as usize)
        .await;

    let Ok(stored) = state.library.download_list() else {
        return;
    };

    for row in stored {
        if row.status.is_finished() {
            continue;
        }

        let paused = row.status == Status::Paused;

        if state.downloads.enqueue(row.request.clone()).await.is_err() {
            continue;
        }

        if paused {
            let _ = state.downloads.pause(&row.request.id).await;
        }
    }
}

// ------------------------------------------------------------------ Releases

/// Put one specific release of one item into the queue.
///
/// **This does not break the rule at the top of this module.** The webview
/// names three integers and a kind; the URL comes from the API and the
/// destination from the user's configured download folder. There is still no
/// `download_start(url, path)`, and there is still nothing here an injected
/// script could point at a file of its choosing.
///
/// Why it exists: an item's page lists every release, and the button beside
/// each one used to hand the URL to the system browser. That is a strange thing
/// for an app whose whole download story — pause, resume, priorities, bandwidth
/// limits, a speed graph, resumable `.tmcpart` files — is the queue this
/// bypasses. Somebody fetching an older release of a mod because the newest one
/// broke their save is exactly the person who wants it in the queue.
///
/// The subscription flow is unchanged and is still what "install this" means.
/// This is the other thing: get me *that* file.
#[tauri::command]
pub async fn download_release(
    state: State<'_, AppState>,
    kind: String,
    item_id: i64,
    release_id: i64,
) -> AppResult<String> {
    /*
     * Re-fetched rather than taken from the caller. The frontend has this
     * payload on screen already, so passing it would save a request — and would
     * also mean the URL that gets downloaded is one the webview supplied, which
     * is the entire thing this command is shaped to avoid.
     */
    let detail = state
        .api
        .request(
            tmc_core::api::Method::GET,
            &format!("/content/{}/{}", kind_segment(&kind)?, item_id),
            None,
            false,
        )
        .await?;

    let release = detail
        .get("releases")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|r| r.get("id").and_then(serde_json::Value::as_i64) == Some(release_id))
        .ok_or_else(|| AppError::invalid("That release is no longer available."))?;

    let file = release
        .get("files")
        .and_then(serde_json::Value::as_array)
        .and_then(|files| files.first())
        .ok_or_else(|| AppError::invalid("That release has no file to download."))?;

    let url = file
        .get("url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::invalid("That release has no file to download."))?;

    let name = detail
        .get("summary")
        .and_then(|s| s.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Download");

    let version = release
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    let file_name = safe_name(
        file.get("name").and_then(serde_json::Value::as_str),
        url,
        &kind,
        item_id,
        release_id,
    );

    let dir = state.download_target_dir();

    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::internal(format!("could not prepare the download folder: {e}")))?;

    /*
     * Deterministic, so pressing the button twice is one row rather than two
     * writers for one file — the same reason a plugin's `download` step derives
     * its id from the plugin and the destination.
     */
    let id = format!("release:{kind}:{item_id}:{release_id}");

    let mut meta = std::collections::BTreeMap::from([
        ("kind".to_string(), kind.clone()),
        // `kind:itemId`, which is what the local library and a sandbox's mods
        // are keyed by — so the queue row finds the item's own artwork without
        // a request.
        ("item".to_string(), format!("{kind}:{item_id}")),
    ]);

    if let Some(version) = &version {
        meta.insert("version".to_string(), version.clone());
    }

    audit!(
        state.audit,
        Info,
        Network,
        "download.release",
        format!(
            "{name} {} → {}",
            version.as_deref().unwrap_or(""),
            file_name
        )
    );

    state
        .downloads
        .enqueue(tmc_core::download::DownloadRequest {
            id: id.clone(),
            url: url.to_string(),
            dest: dir.join(&file_name),
            label: match &version {
                Some(v) => format!("{name} {v}"),
                None => name.to_string(),
            },
            sha256: file
                .get("sha256")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            size_hint: file
                .get("size")
                .and_then(serde_json::Value::as_i64)
                .and_then(|n| u64::try_from(n).ok()),
            priority: 0,
            limit_bps: None,
            meta,
        })
        .await?;

    Ok(id)
}

/// The kind, as a path segment.
///
/// A closed list rather than a sanitiser. This value reaches a URL path, and
/// the difference between "escape it carefully" and "it is one of these six"
/// is the difference between a check somebody can get subtly wrong later and
/// one they cannot.
fn kind_segment(kind: &str) -> AppResult<&'static str> {
    match kind {
        "mod" => Ok("mod"),
        "asset" => Ok("asset"),
        "collection" => Ok("collection"),
        "article" => Ok("article"),
        "server" => Ok("server"),
        "community" => Ok("community"),
        _ => Err(AppError::invalid("That is not a content kind.")),
    }
}

/// A single, safe file name for a release file.
///
/// Every character outside a plain name alphabet becomes `-`, `..` cannot
/// survive, and a name that sanitises to nothing falls back to the ids. The
/// destination directory is the app's own, but a release file name is
/// author-written text off the network and `Path::join` with a `../` in it
/// would land outside.
fn safe_name(
    declared: Option<&str>,
    url: &str,
    kind: &str,
    item_id: i64,
    release_id: i64,
) -> String {
    let raw = declared
        .filter(|n| !n.is_empty())
        .or_else(|| url.rsplit('/').next().filter(|s| !s.is_empty()))
        .unwrap_or_default();

    // The last component only, and only up to a query string.
    let base = raw
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();

    let mut out = String::with_capacity(base.len());
    let mut last_dot = false;

    for ch in base.chars() {
        let keep = ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '+' | '(' | ')');

        if !keep {
            last_dot = false;
            out.push('-');

            continue;
        }

        if ch == '.' && last_dot {
            continue;
        }

        last_dot = ch == '.';
        out.push(ch);
    }

    let trimmed = out.trim_matches(|c: char| c == '.' || c == '-').to_string();

    if trimmed.is_empty() || trimmed.len() > 180 {
        return format!("{kind}-{item_id}-{release_id}.bin");
    }

    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_file_name_cannot_escape_the_download_folder() {
        for hostile in [
            "../../etc/passwd",
            "..\\..\\windows\\system32\\evil.dll",
            "/etc/shadow",
            "....//....//x",
            "a\0b",
        ] {
            let name = safe_name(Some(hostile), "https://x.test/f", "mod", 1, 2);

            assert!(!name.contains('/'), "{hostile} → {name}");
            assert!(!name.contains('\\'), "{hostile} → {name}");
            assert!(!name.contains(".."), "{hostile} → {name}");
            assert!(!name.contains('\0'), "{hostile} → {name}");
        }
    }

    #[test]
    fn an_ordinary_name_survives_intact() {
        assert_eq!(
            safe_name(Some("cool-mod_1.4.2.zip"), "https://x.test/f", "mod", 1, 2),
            "cool-mod_1.4.2.zip"
        );
    }

    /// No declared name: the URL's last component, without its query string.
    #[test]
    fn the_url_is_the_fallback_and_its_query_is_not_part_of_the_name() {
        assert_eq!(
            safe_name(
                None,
                "https://cdn.test/files/pack.zip?token=abc",
                "mod",
                1,
                2
            ),
            "pack.zip"
        );
    }

    #[test]
    fn a_name_that_sanitises_to_nothing_falls_back_to_the_ids() {
        assert_eq!(
            safe_name(Some("..."), "https://x.test/", "mod", 7, 9),
            "mod-7-9.bin"
        );
        assert_eq!(safe_name(None, "", "asset", 7, 9), "asset-7-9.bin");
    }

    /// The kind reaches a URL path, so it is a closed list rather than an
    /// escaped string.
    #[test]
    fn only_known_kinds_reach_the_api_path() {
        assert!(kind_segment("mod").is_ok());
        assert!(kind_segment("../admin").is_err());
        assert!(kind_segment("mod/../..").is_err());
        assert!(kind_segment("").is_err());
    }
}
