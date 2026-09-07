//! **Playing a game**, in the two senses the app can mean it.
//!
//! There are three ways a game starts from here and only two of them are in
//! this module:
//!
//!   * **In a window** — an app's uploaded JavaScript loader, run in a webview
//!     of its own. [`play_open_web`].
//!   * **A hand-off** — the site's `playAppUri` resolved to a URI the OS opens.
//!     [`play_handoff`]. Rare, and it exists for completeness: that URI is
//!     normally `tmc://`, which addresses this process.
//!   * **The copy installed on this machine**, which is not here at all. That
//!     is a sandbox launch, it goes through `commands::sandbox`, and the server
//!     has no way of knowing whether it is possible — so it is not the server's
//!     to declare and not this module's to resolve.
//!
//! WHY THE WEBVIEW NAMES IDS AND NOTHING ELSE
//! -----------------------------------------
//! A loader URL from the frontend would be a script URL injected into a page on
//! the SITE's origin. So the same rule `download_release` follows applies here:
//! the caller names an app id and, optionally, a server id, and Rust asks the
//! API what that resolves to. There is no `play_open(loaderUrl)`, and nothing
//! an injected script could point at a script of its choosing.
//!
//! WHY THE PLAYER IS A REMOTE PAGE
//! ------------------------------
//! The window opens at `<site>/app-player` rather than at a route in this app's
//! own bundle, and that is the isolation the whole feature rests on:
//!
//!   * **The app's CSP forbids it here.** `script-src 'self'` means a loader on
//!     a CDN does not execute in this app's webview at all, and widening that
//!     would widen it for the one webview that holds the IPC surface.
//!   * **A remote page has no IPC.** Tauri exposes commands to the app's own
//!     asset origin only; a window pointed at `https://` has no `invoke`, no
//!     event channel and no plugin access — enforced by the runtime rather than
//!     by a permission list somebody has to keep correct. A hostile loader is a
//!     web page with a web page's powers, and cannot reach the sandbox engine,
//!     the RCON store or the settings that anchor the plugin jail.
//!
//! The descriptor reaches the page through an initialisation script rather than
//! a query string, for the reason the site's own player gives: a server address
//! on a URL ends up in every cache entry and history record that URL touches.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

use tmc_core::error::{AppError, AppResult};
use tmc_core::session::{Session, SessionSpec};

use crate::state::AppState;

/// The player window's label.
///
/// Fixed rather than per-session, which makes the window single-instance by
/// construction: a second Play press reuses it. Two loaders running at once
/// would compete for the machine's GPU while neither is what the user is
/// looking at, and closing the "wrong" one is a puzzle nobody should be given.
const PLAYER_LABEL: &str = "game-player";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRequest {
    pub app_id: i64,
    /// Launch into a specific server. Required for an app without
    /// `directPlay`, and the server enforces that rather than trusting us.
    #[serde(default)]
    pub server_id: Option<i64>,
    /// The values the user chose for the app's declared launch options.
    ///
    /// Passed through verbatim and coerced SERVER-side against the app's own
    /// declaration — an unknown key is dropped there, not here. Validating in
    /// two places is how the two get different answers.
    #[serde(default)]
    pub options: Option<serde_json::Value>,
    /// What to put on the window while it loads.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub app_slug: Option<String>,
}

/// What `/play/launch` answered, as far as this module cares.
#[derive(Debug, Deserialize)]
struct Resolved {
    mode: String,
    #[serde(default)]
    loader_url: Option<String>,
    #[serde(default)]
    boot: Option<serde_json::Value>,
    #[serde(default)]
    uri: Option<String>,
}

/// Ask the site to resolve a launch.
///
/// `null` is its whole refusal vocabulary — the app is hidden, the feature is
/// off, the mode is unavailable, or `directPlay` is off and no server was
/// named. It is turned into one sentence here because none of those change what
/// the app does, and enumerating them would tell a caller which column to probe.
async fn resolve(state: &AppState, mode: &str, request: &PlayRequest) -> AppResult<Resolved> {
    let payload = serde_json::json!({
        "appId": request.app_id,
        "mode": mode,
        "serverId": request.server_id,
        "options": request.options,
    });

    let answer = state
        .api
        .request(
            tmc_core::api::Method::POST,
            "/play/launch",
            Some(payload),
            false,
        )
        .await?;

    if answer.is_null() {
        return Err(AppError::invalid(
            "This game cannot be started from here. Open its page to see where it can be.",
        ));
    }

    serde_json::from_value::<Resolved>(answer)
        .map_err(|e| AppError::internal(format!("play/launch: {e}")))
}

#[tauri::command]
pub async fn play_open_web(
    app: AppHandle,
    state: State<'_, AppState>,
    request: PlayRequest,
) -> AppResult<Session> {
    let resolved = resolve(&state, "web", &request).await?;

    if resolved.mode != "web" {
        return Err(AppError::internal("play/launch answered the wrong mode"));
    }

    let (Some(loader_url), Some(boot)) = (resolved.loader_url, resolved.boot) else {
        return Err(AppError::internal("play/launch answered without a loader"));
    };

    /*
     * Checked even though it came from our own API. It is about to become a
     * `<script src>` on the site's origin, and "the server would never send
     * that" is a claim about a server, not a property of the string in hand.
     */
    if !loader_url.starts_with("https://") {
        return Err(AppError::invalid(
            "That game's loader is not served over HTTPS and will not be run.",
        ));
    }

    let title = request
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "TMC Player".to_string());

    let url = format!(
        "{}/app-player",
        tmc_core::api::api_base().trim_end_matches('/')
    );

    let parsed = url
        .parse()
        .map_err(|_| AppError::internal("the site's own URL did not parse"))?;

    /*
     * The descriptor is injected before the page's own scripts run, which is
     * what makes the query string unnecessary. `serde_json::to_string` is what
     * escapes it — every value in here came off the wire, and building this
     * with `format!` would be a script-injection sink one unusual server name
     * away.
     */
    let bootstrap = format!(
        "window.__TMC_GAME__ = {}; window.__TMC_PLAYER__ = {};",
        serde_json::to_string(&boot)
            .map_err(|e| AppError::internal(format!("boot descriptor: {e}")))?,
        serde_json::to_string(&serde_json::json!({
            "loaderUrl": loader_url,
            "title": title,
        }))
        .map_err(|e| AppError::internal(format!("player handoff: {e}")))?,
    );

    // A window already open is closed rather than reused. Re-navigating would
    // leave the previous loader's timers and audio contexts alive in the same
    // document, which is how a second game starts with the first still playing.
    if let Some(existing) = app.get_webview_window(PLAYER_LABEL) {
        let _ = existing.close();
    }

    /*
     * Everything that can fail happens BEFORE the session is opened.
     *
     * It used to be opened first, and every error path after it — a URL that
     * would not parse, a descriptor that would not serialise, a window that
     * would not build — left a live session with no window and no destroy
     * handler to close it. Nothing could ever end that session: it showed as a
     * running game forever, and the Library's Stop button routes a web session
     * to `play_close`, which finds no window and cheerfully returns Ok.
     */
    let built = WebviewWindowBuilder::new(&app, PLAYER_LABEL, WebviewUrl::External(parsed))
        .title(&title)
        .inner_size(1280.0, 800.0)
        .min_inner_size(640.0, 400.0)
        .resizable(true)
        .center()
        /*
         * DECORATED, unlike the main window.
         *
         * The main window draws its own frame because Tauri's Linux backend is
         * WebKitGTK and a decorated window there gets a GTK titlebar themed by
         * whatever desktop the user runs. That reasoning does not carry here:
         * drawing our own frame needs `core:window:allow-start-dragging`, and
         * this window is a remote page precisely so that it has no IPC at all.
         * A native frame is the price of that isolation, and it is the right
         * one — a game window with its own OS controls is what people expect.
         */
        .decorations(true)
        .initialization_script(&bootstrap)
        .build()
        .map_err(|e| AppError::internal(format!("could not open the player: {e}")))?;

    let session = state.sessions.open_web(SessionSpec {
        app_id: Some(request.app_id),
        app_slug: request.app_slug.clone(),
        label: title.clone(),
        sandbox_id: None,
        // A browser game is not an install, so there is nothing on the account
        // to report its play time against. It is still measured locally.
        install_id: None,
    });

    /*
     * The session ends when the window does, and Rust is what notices.
     *
     * Leaving it to the frontend would mean a session that stays open when the
     * app is closed with a game running, and a play clock that keeps counting
     * against a window nobody can see.
     */
    {
        let sessions = std::sync::Arc::clone(&state.sessions);
        let id = session.id;

        built.on_window_event(move |event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                sessions.end(id, false);
            }
        });
    }

    /*
     * A window destroyed between `build` and the handler being attached would
     * leak the session the same way. Cheap to rule out, and the race is real on
     * a machine slow enough for the page to fail while this function runs.
     */
    if app.get_webview_window(PLAYER_LABEL).is_none() {
        state.sessions.end(session.id, false);

        return Err(AppError::internal(
            "The player window closed before it finished opening.",
        ));
    }

    tmc_core::audit!(
        state.audit,
        Info,
        App,
        "play.web",
        format!("opened the player for {title}")
    );

    Ok(session)
}

/// Resolve the site's `playAppUri` and hand it to the OS.
///
/// Present for completeness rather than for daily use: that template normally
/// produces a `tmc://` link, which addresses the process reading it. It earns
/// its place for an app whose URI names a DIFFERENT client — a game with its
/// own launcher protocol — which is a thing the column can express and nothing
/// else here could act on.
#[tauri::command]
pub async fn play_handoff(
    app: AppHandle,
    state: State<'_, AppState>,
    request: PlayRequest,
) -> AppResult<Session> {
    let resolved = resolve(&state, "app", &request).await?;

    let Some(uri) = resolved.uri else {
        return Err(AppError::internal("play/launch answered without a URI"));
    };

    /*
     * The site refuses `javascript:`, `data:`, `vbscript:`, `blob:` and `file:`
     * when the template is written and again when it is read. Re-checked here
     * because this is the process that would actually open it, and a check that
     * lives only on the other side of a network hop is a check this process is
     * trusting somebody else to have run.
     */
    let lowered = uri.trim().to_ascii_lowercase();

    if ["javascript:", "data:", "vbscript:", "blob:", "file:"]
        .iter()
        .any(|bad| lowered.starts_with(bad))
    {
        return Err(AppError::invalid(
            "That game's launch link uses a scheme the app will not open.",
        ));
    }

    use tauri_plugin_opener::OpenerExt;

    app.opener()
        .open_url(uri.clone(), None::<&str>)
        .map_err(|e| AppError::internal(format!("could not open {uri}: {e}")))?;

    tmc_core::audit!(state.audit, Security, App, "play.handoff", uri.clone());

    Ok(state.sessions.record_handoff(SessionSpec {
        app_id: Some(request.app_id),
        app_slug: request.app_slug.clone(),
        label: request.title.clone().unwrap_or_else(|| "Game".to_string()),
        sandbox_id: None,
        install_id: None,
    }))
}

/// Hand a server's own connect link to the installed game client.
///
/// **The webview names a server id and nothing else.** The URL is read from the
/// API here, for the same reason the loader URL is: "open this URI" hands a
/// string to whatever program claimed a scheme, and a caller-supplied one is a
/// caller choosing which program runs.
///
/// The scheme is checked against [`tmc_core::plugins::apps::LAUNCH_SCHEMES`],
/// the same closed list an app plugin's launch rule is held to. It does NOT go
/// through `tauri-plugin-opener`'s own capability, which is scoped to
/// `https://*` — a `steam://` link is exactly what that scope exists to keep
/// the webview from opening on its own.
#[tauri::command]
pub async fn play_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    server_id: i64,
) -> AppResult<Session> {
    let detail = state
        .api
        .request(
            tmc_core::api::Method::GET,
            &format!("/content/server/{server_id}"),
            None,
            false,
        )
        .await?;

    let summary = detail.get("summary").unwrap_or(&detail);

    let uri = summary
        .get("server")
        .and_then(|s| s.get("connectUrl"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();

    if uri.is_empty() {
        return Err(AppError::invalid(
            "That server publishes no connect link for its game.",
        ));
    }

    if !tmc_core::plugins::apps::is_allowed_launch_uri(&uri) {
        return Err(AppError::invalid(
            "That server's connect link does not name a game client the app will open.",
        ));
    }

    use tauri_plugin_opener::OpenerExt;

    app.opener()
        .open_url(uri.clone(), None::<&str>)
        .map_err(|e| AppError::internal(format!("could not open {uri}: {e}")))?;

    tmc_core::audit!(state.audit, Security, App, "play.connect", uri.clone());

    let app_id = summary
        .get("app")
        .and_then(|a| a.get("id"))
        .and_then(serde_json::Value::as_i64);

    let label = summary
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Server")
        .to_string();

    Ok(state.sessions.record_handoff(SessionSpec {
        app_id,
        app_slug: None,
        label,
        sandbox_id: None,
        install_id: None,
    }))
}

/// Close the player window, if it is open.
///
/// Separate from `session_close_web` because they answer different halves: this
/// closes a WINDOW, and the session ends as a consequence of it closing. Doing
/// it the other way round would leave a window on screen whose session had
/// already been banked.
#[tauri::command]
pub fn play_close(app: AppHandle) -> AppResult<()> {
    if let Some(window) = app.get_webview_window(PLAYER_LABEL) {
        window
            .close()
            .map_err(|e| AppError::internal(format!("could not close the player: {e}")))?;
    }

    Ok(())
}

/// Whether the player window is open, so a restored screen agrees with reality.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerState {
    pub open: bool,
}

#[tauri::command]
pub fn play_state(app: AppHandle) -> PlayerState {
    PlayerState {
        open: app.get_webview_window(PLAYER_LABEL).is_some(),
    }
}
