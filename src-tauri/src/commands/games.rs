//! **Games TMC publishes**: installing them, keeping them current, starting
//! them.
//!
//! The command surface over [`tmc_core::games`]. Everything the Library's
//! "TMC Games" section can do is here, and the two rules at the top of
//! `commands/mod.rs` hold without an exception:
//!
//!   * **Nothing here takes a path.** The install root comes from Tauri's path
//!     resolver, the version directory from the build the SITE resolved, and
//!     the executable from that build's own declaration put through the jail's
//!     lexical check. A caller names an app id.
//!   * **Nothing here takes a URL.** `game_install` asks the API where the
//!     build is; there is no `game_install(url)`, for the same reason there is
//!     no `download_start(url, path)`.
//!
//! WHY A LAUNCH NAMES A SERVER ID AND NOT AN ADDRESS
//! ------------------------------------------------
//! The address is read from the API here, as `play_connect` already does it.
//! A caller-supplied `host:port` would be a caller choosing what a process this
//! app starts is pointed at — and while a game client dialling a stranger's box
//! is a smaller problem than a plugin writing to one, it is the same shape, and
//! the address is one lookup away.
//!
//! WHY AN INSTALL IS NOT AWAITED BY THE UI
//! --------------------------------------
//! It is: `game_install` runs to completion and the dialog awaits it. The
//! PROGRESS, though, comes from the download queue's own events — the build is
//! enqueued like every other download, so the existing queue screen shows it,
//! pauses it and rate-limits it without learning what a game is.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use tmc_core::audit;
use tmc_core::error::{AppError, AppResult};
use tmc_core::games::{BuildPlatform, GameStatus, LaunchContext, NativeBuild};
use tmc_core::library::db::InstalledGame;
use tmc_core::session::{Session, SessionSpec};

use crate::state::AppState;

/// What this machine can install, so the UI can say why it cannot.
///
/// Two separate facts, deliberately. "This app publishes nothing for your
/// architecture" can never change for the person reading it; "games are
/// installed through your platform's own store" is a statement about Android
/// and iOS refusing to execute what an app wrote into its own container. A
/// single `false` would have made those one message, and neither of the two
/// sentences that message could carry would be true for both.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GamePlatformInfo {
    /// The target this machine asks for, or null for one nothing is built for.
    pub platform: Option<String>,
    /// Whether the app can put a build on this disk and start it.
    pub installable: bool,
}

#[tauri::command]
pub fn games_platform(state: State<'_, AppState>) -> GamePlatformInfo {
    let platform = state.games.platform();

    GamePlatformInfo {
        platform: platform.map(BuildPlatform::as_str).map(str::to_string),
        installable: platform.is_some_and(BuildPlatform::installable),
    }
}

/// Everything installed on this device, newest information first.
#[tauri::command]
pub fn games_list(state: State<'_, AppState>) -> AppResult<Vec<InstalledGame>> {
    state.games.installed()
}

/// What the site has for this machine, or null for "nothing to install".
#[tauri::command]
pub async fn games_available(
    state: State<'_, AppState>,
    app_id: i64,
) -> AppResult<Option<NativeBuild>> {
    state.games.resolve(app_id).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRequest {
    pub app_id: i64,
    /// What to call it while it downloads. A LABEL — the row's real name comes
    /// from the catalogue, and nothing is keyed on this.
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
}

/// Install, or update in place. One command, because an update is an install
/// whose row already exists — see `Games::install`.
#[tauri::command]
pub async fn game_install(
    state: State<'_, AppState>,
    request: InstallRequest,
) -> AppResult<InstalledGame> {
    /*
     * Refused while it is running. The new files would land on top of ones the
     * running process has mapped, which on Windows fails outright and on Unix
     * succeeds and leaves a process running code that is no longer on disk.
     */
    if state.sessions.running_for_app(request.app_id) {
        return Err(AppError::invalid(
            "That game is running. Close it before installing an update.",
        ));
    }

    state
        .games
        .install(request.app_id, &request.name, request.slug.as_deref())
        .await
}

#[tauri::command]
pub async fn game_uninstall(state: State<'_, AppState>, app_id: i64) -> AppResult<()> {
    if state.sessions.running_for_app(app_id) {
        return Err(AppError::invalid(
            "That game is running. Close it before removing it.",
        ));
    }

    state.games.uninstall(app_id)
}

#[tauri::command]
pub fn game_set_auto_update(
    state: State<'_, AppState>,
    app_id: i64,
    enabled: bool,
) -> AppResult<()> {
    state.games.set_auto_update(app_id, enabled)
}

/// Check every installed game against the site, and say what is out of date.
///
/// Reports rather than acts, even for a game with auto-update on. The automatic
/// pass is `games::updates::run`, which the sync loop drives; this is the button
/// somebody presses, and a button that silently downloaded four gigabytes
/// because the row happened to have a flag set would be a button nobody presses
/// twice.
#[tauri::command]
pub async fn games_check_updates(state: State<'_, AppState>) -> AppResult<Vec<GameStatus>> {
    tmc_core::games::updates::check(&state.games).await
}

/// Bring every installed game that asked to be kept current forward.
///
/// Called on the library's sync pass, which is the device's own "am I online
/// now?" heartbeat — the same place outstanding play time is drained, and for
/// the same reason: it is already the moment the device has an API to talk to.
///
/// Bounded by the rules in `games::updates`: the install must have asked for
/// this, the game must not be running, and a version that cannot be ordered is
/// never an update. A game that fails leaves the version it already had on disk
/// and working, because the new build lands in its own directory and the row is
/// only repointed once the entry is verified.
#[tauri::command]
pub async fn games_auto_update(
    state: State<'_, AppState>,
) -> AppResult<tmc_core::games::updates::GameUpdateReport> {
    let sessions = std::sync::Arc::clone(&state.sessions);

    tmc_core::games::updates::run(&state.games, &move |app_id| {
        sessions.running_for_app(app_id)
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameLaunchRequest {
    pub app_id: i64,
    /// Join this server. Absent starts the game at its own menu.
    #[serde(default)]
    pub server_id: Option<i64>,
    /// The values the user chose for the app's declared launch options.
    #[serde(default)]
    pub options: Option<serde_json::Value>,
    /// The viewer's language tag, for a build whose arguments name `{locale}`.
    ///
    /// From the caller because locale is an ACCOUNT setting and this side has
    /// none — the app-local settings are the machine facts, deliberately. It is
    /// a display string that goes through the same control-character check
    /// every other substituted value does, so the worst a wrong one produces is
    /// a game in the wrong language.
    #[serde(default)]
    pub locale: Option<String>,
    /// A game pack to run with no server (`<owner>/<repo>`), from a
    /// `tmc://play/app/..?solo=` link. Only a fallback: the site's own
    /// `boot.solo` wins. Ignored when a server is named.
    #[serde(default)]
    pub solo: Option<String>,
    #[serde(default)]
    pub solo_version: Option<String>,
}

/// Start an installed game, optionally into a server.
#[tauri::command]
pub async fn game_launch(
    app: AppHandle,
    state: State<'_, AppState>,
    request: GameLaunchRequest,
) -> AppResult<Session> {
    let target = native_target(
        &*state.api,
        |id| state.games.get(id),
        request.app_id,
        request.locale.as_deref(),
    )
    .await?;

    let (row, game) = match target {
        NativeTarget::Own(row) => (row, None),
        NativeTarget::Engine { row, game } => (row, Some(game)),
        NativeTarget::EngineMissing { .. } | NativeTarget::Nothing => {
            return Err(AppError::invalid("That game is not installed."))
        }
    };

    /*
     * A second copy would open files the first has mapped, and the play clock
     * for one game would be able to exceed wall-clock time. The same refusal a
     * sandbox launch carries, for the same two reasons. Keyed on the INSTALL's
     * app, so a game started through its engine's build counts as that build
     * running: the update and uninstall refusals ask by the same id.
     */
    if state.sessions.running_for_app(row.app_id) {
        return Err(AppError::invalid("That game is already running."));
    }

    let mut ctx = LaunchContext {
        app_id: request.app_id,
        app_slug: game
            .as_ref()
            .map_or_else(|| row.slug.clone(), |g| g.slug.clone().or(row.slug.clone())),
        locale: request.locale.clone(),
        ..Default::default()
    };

    if let Some(server_id) = request.server_id {
        let (host, port, udp_port) = resolve_server(&state, server_id).await?;

        ctx.server_id = Some(server_id);
        ctx.host = Some(host);
        ctx.port = port;
        ctx.udp_port = udp_port;
    }

    if let Some(serde_json::Value::Object(map)) = request.options {
        for (key, value) in map {
            let text = match value {
                serde_json::Value::String(s) => s,
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                // An object or an array contributes nothing, exactly as it
                // does to a sandbox's `optionArgs`. There is no shell and no
                // serialisation a game could be relied on to parse.
                _ => continue,
            };

            ctx.options.insert(key, text);
        }
    }

    let mut plan = state.games.plan(&row, &ctx)?;

    // Where the game keeps its downloads, and whether it keeps them at all --
    // the app's folder, never a path a caller or a setting names.
    let settings = state.settings.get();
    plan.args.extend(tmc_core::games::build::cache_args(
        &plan.args,
        settings.game_cache_keep,
        &game_cache_dir(&state),
        settings.game_cache_limit_mb,
    ));
    // The site whose servers the in-game browser lists. An explicit flag or
    // DOT_BROWSER_BACKBONE_URL still wins (see the function).
    plan.args.extend(tmc_core::games::build::browser_site_args(
        &plan.args,
        tmc_core::api::api_base(),
        std::env::var_os("DOT_BROWSER_BACKBONE_URL").is_some_and(|v| !v.is_empty()),
    ));
    // No server: a game pack app runs its own offline mode ("Solo play"). The
    // site names the pack in the boot descriptor; asking is best effort, so a
    // site that is unreachable, older than the field, or refuses just starts
    // the game at its own menu as before.
    if request.server_id.is_none() {
        let linked = request.solo.as_deref().and_then(|id| {
            tmc_core::games::build::SoloPack::new(id, request.solo_version.as_deref())
        });

        let boot = launch_boot(&state, request.app_id, request.locale.as_deref()).await;
        let resolved = boot
            .as_ref()
            .and_then(tmc_core::games::build::SoloPack::from_boot);

        if let Some(solo) = resolved.or(linked) {
            // Where the pack comes from: a solo shell has no server to tell it.
            let base = boot
                .as_ref()
                .and_then(tmc_core::games::build::content_url_from_boot);
            plan.args.extend(tmc_core::games::build::content_url_args(
                &plan.args,
                base.as_deref(),
            ));
            // No bare `--` in the published template: the shell would start at
            // its own connect screen. Say so rather than opening it silently.
            let solo_args = match tmc_core::games::build::solo_args_or_why(&plan.args, &solo) {
                Ok(args) => args,
                Err(why) => {
                    tmc_core::audit!(state.audit, Error, App, "game.launch.solo", why);

                    return Err(AppError::invalid(why));
                }
            };

            plan.args.extend(solo_args);
        } else {
            tmc_core::audit!(
                state.audit,
                Warn,
                App,
                "game.launch.solo",
                "No solo pack from /play/launch (no boot.solo) and no link fallback; the game starts at its own menu."
            );
        }
    }
    // Signed in as themselves: a 60 s single-use handoff the site mints for
    // this launch, redeemed by the shell at its own backbone. Best effort; any
    // failure leaves the guest launch this was before.
    if let Some(code) = launch_auth_code(&state, request.app_id, request.server_id).await {
        plan.args.extend(tmc_core::games::build::auth_code_args(
            &plan.args,
            Some(code.as_str()),
            tmc_core::api::api_base(),
        ));
    }
    // UDP or not: the player's choice for a network that drops it.
    plan.args.extend(tmc_core::games::build::transport_args(
        &plan.args,
        settings.game_use_udp,
    ));

    tmc_core::audit!(
        state.audit,
        Info,
        App,
        "game.launch",
        format!("app {} args: {}", row.app_id, plan.args.join(" "))
    );

    let session = crate::spawn::run(
        &app,
        &state,
        &plan,
        SessionSpec {
            // The INSTALL's id, for the same reason as the refusal above; the
            // slug and label are the game's, so its play time and history
            // rows read as the game rather than as the engine it ran on.
            app_id: Some(row.app_id),
            app_slug: ctx.app_slug.clone(),
            label: game
                .and_then(|g| g.name)
                .unwrap_or_else(|| row.name.clone()),
            sandbox_id: None,
            // A TMC game is not a subscription, so there is no install row on
            // the account to report its play time against. It is still measured
            // locally and still written to `game_session`.
            install_id: None,
        },
    )?;

    Ok(session)
}

/// The game a launch named, when it runs on its engine's build.
#[derive(Debug, PartialEq)]
struct EngineGame {
    engine_id: i64,
    slug: Option<String>,
    name: Option<String>,
}

/// Which engine app `app_id` runs on, read from a `/apps?ids=` answer.
///
/// `None` when the answer is not about that app (an id the catalogue does not
/// list returns some other row or none), when it names no engine, or when it
/// names itself, which is an engine's own row rather than a game on one.
fn engine_of(answer: &serde_json::Value, app_id: i64) -> Option<EngineGame> {
    let app = answer
        .get("apps")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .find(|a| a.get("id").and_then(serde_json::Value::as_i64) == Some(app_id))?;

    let engine_id = app
        .get("engine")
        .and_then(|e| e.get("id"))
        .and_then(serde_json::Value::as_i64)
        .filter(|id| *id != app_id)?;

    Some(EngineGame {
        engine_id,
        slug: app
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .map(str::to_ascii_lowercase),
        name: app
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    })
}

/// What pressing Play on one app would start on this machine.
#[derive(Debug)]
enum NativeTarget {
    /// The app's own installed build.
    Own(InstalledGame),
    /// A TMC pack game, on its engine's installed build.
    Engine {
        row: InstalledGame,
        game: EngineGame,
    },
    /// A TMC pack game whose engine's build is not installed.
    EngineMissing { engine_id: i64 },
    /// Nothing here runs it.
    Nothing,
}

/// The two catalogue questions [`native_target`] asks, so its tests can answer
/// them without a site.
trait Catalogue {
    /// `GET /apps?ids=`: which engine the app names.
    async fn app(&self, app_id: i64) -> AppResult<serde_json::Value>;
    /// `POST /play/launch` for a serverless web launch, asked anonymously.
    async fn solo_boot(&self, app_id: i64, locale: Option<&str>) -> AppResult<serde_json::Value>;
}

impl Catalogue for tmc_core::api::ApiClient {
    async fn app(&self, app_id: i64) -> AppResult<serde_json::Value> {
        self.request(
            tmc_core::api::Method::GET,
            &format!("/apps?ids={app_id}&limit=1"),
            None,
            false,
        )
        .await
    }

    /*
     * Anonymous on purpose. The route counts a signed-in launch as a play and
     * may mint a handoff, and this is asked when the Play dialog opens, before
     * anybody has pressed anything. Nothing it decides about a pack depends on
     * who is asking (`ResolveLaunchSolo` reads the app's pack and nothing else).
     */
    async fn solo_boot(&self, app_id: i64, locale: Option<&str>) -> AppResult<serde_json::Value> {
        self.anonymous_post(
            "/play/launch",
            serde_json::json!({
                "appId": app_id,
                "mode": "web",
                "locale": locale.unwrap_or("en"),
            }),
        )
        .await
    }
}

const CATALOGUE_UNREACHABLE: &str =
    "Couldn't reach the catalogue to find this game's TMC build. Check your connection.";

/// Which installed build runs `app_id`: its own, or its engine's.
///
/// A TMC game is a pack the shell downloads, and the shell is published once,
/// under the engine app (dot-server-deploy's `export-native`), because a server
/// can change which game it runs while its players stay connected. So Arena has
/// no build and Godot does, and pressing Play on Arena means starting Godot's
/// build with Arena's pack (`boot.solo`) or Arena's server.
///
/// **Only a TMC pack game falls back.** A third-party game made with the same
/// engine names the same engine, and starting the TMC shell for it would put the
/// player at the shell's menu, or connect the shell to a server it cannot play.
/// The site is what knows: it names a signed pack in `boot.solo` only for an app
/// with one, so the fallback asks `/play/launch` and needs `boot.solo` back. The
/// catalogue row has no such flag. The engine is read from the catalogue, never
/// named by the caller, so the webview names one app id and Rust decides what
/// runs.
///
/// An unreachable catalogue is an error rather than `Nothing`: "not installed"
/// would send somebody who has the build to install it again.
async fn native_target(
    site: &impl Catalogue,
    installed: impl Fn(i64) -> AppResult<Option<InstalledGame>>,
    app_id: i64,
    locale: Option<&str>,
) -> AppResult<NativeTarget> {
    if let Some(row) = installed(app_id)? {
        return Ok(NativeTarget::Own(row));
    }

    let answer = site.app(app_id).await.map_err(catalogue_unreachable)?;

    let Some(game) = engine_of(&answer, app_id) else {
        return Ok(NativeTarget::Nothing);
    };

    let boot = match site.solo_boot(app_id, locale).await {
        Ok(answer) => answer.get("boot").cloned(),
        Err(e @ (AppError::Network(_) | AppError::Internal(_))) => {
            return Err(catalogue_unreachable(e))
        }
        // Refused (play off, a hidden app): not a launch this can make.
        Err(_) => None,
    };

    if boot
        .as_ref()
        .and_then(tmc_core::games::build::SoloPack::from_boot)
        .is_none()
    {
        return Ok(NativeTarget::Nothing);
    }

    Ok(match installed(game.engine_id)? {
        Some(row) => NativeTarget::Engine { row, game },
        None => NativeTarget::EngineMissing {
            engine_id: game.engine_id,
        },
    })
}

fn catalogue_unreachable(e: AppError) -> AppError {
    tracing::debug!("catalogue lookup for a native build failed: {e}");
    AppError::Network(CATALOGUE_UNREACHABLE.into())
}

/// What the Play dialog offers natively for one app.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeTargetView {
    /// The build Play would start: the app's own, or its engine's.
    pub installed: Option<InstalledGame>,
    /// Set when the app is a TMC pack game played on this engine's build.
    pub engine_id: Option<i64>,
    /// The engine's build is not installed, and the site has one for this
    /// machine to install.
    pub engine_installable: bool,
}

/// The Play dialog's native card and its "Install it" hint, from the same gate
/// [`game_launch`] uses, so the dialog never offers a launch Rust then refuses.
#[tauri::command]
pub async fn game_native_target(
    state: State<'_, AppState>,
    app_id: i64,
    locale: Option<String>,
) -> AppResult<NativeTargetView> {
    let target = native_target(
        &*state.api,
        |id| state.games.get(id),
        app_id,
        locale.as_deref(),
    )
    .await?;

    Ok(match target {
        NativeTarget::Own(row) => NativeTargetView {
            installed: Some(row),
            engine_id: None,
            engine_installable: false,
        },
        NativeTarget::Engine { row, game } => NativeTargetView {
            installed: Some(row),
            engine_id: Some(game.engine_id),
            engine_installable: false,
        },
        NativeTarget::EngineMissing { engine_id } => NativeTargetView {
            installed: None,
            engine_id: Some(engine_id),
            // Best effort: no hint is the answer when the site cannot say.
            engine_installable: state
                .games
                .resolve(engine_id)
                .await
                .ok()
                .flatten()
                .is_some(),
        },
        NativeTarget::Nothing => NativeTargetView {
            installed: None,
            engine_id: None,
            engine_installable: false,
        },
    })
}

/// The address of one server, read here rather than accepted from the caller.
///
/// Null host is the owner having hidden the network details, which is a refusal
/// rather than a launch into nothing: a client started with no address shows its
/// own "could not connect" and the person who pressed Join has no way to tell
/// that from a server being down.
async fn resolve_server(
    state: &AppState,
    server_id: i64,
) -> AppResult<(String, Option<u16>, Option<u16>)> {
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

    let server = summary
        .get("server")
        .ok_or_else(|| AppError::invalid("That is not a server."))?;

    let host = server
        .get("host")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|h| !h.is_empty())
        .ok_or_else(|| {
            AppError::invalid("That server does not publish an address to connect to.")
        })?;

    let port = server
        .get("port")
        .and_then(serde_json::Value::as_u64)
        .and_then(|p| u16::try_from(p).ok());

    // A dual-stack server's ENet port, from its own query; absent on a site
    // older than the field and on every server that advertises none.
    let udp_port = server
        .get("udpPort")
        .and_then(serde_json::Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p > 0);

    Ok((host, port, udp_port))
}

/// The boot descriptor for a serverless launch of this app, from `/play/launch`:
/// `boot.solo` names the pack and `boot.game.origin` where it is served.
async fn launch_boot(
    state: &AppState,
    app_id: i64,
    locale: Option<&str>,
) -> Option<serde_json::Value> {
    let payload = serde_json::json!({
        "appId": app_id,
        "mode": "web",
        "locale": locale.unwrap_or("en"),
    });

    let answer = state
        .api
        .request(
            tmc_core::api::Method::POST,
            "/play/launch",
            Some(payload),
            false,
        )
        .await
        .ok()?;

    answer.get("boot").cloned()
}

/// A sign-in handoff for this launch, from `POST /play/launch` in `app` mode as
/// the signed-in player, or `None` (signed out, a server that does not sign in
/// through TMC, a refusal, a site older than the field, or a network error).
async fn launch_auth_code(
    state: &AppState,
    app_id: i64,
    server_id: Option<i64>,
) -> Option<String> {
    let mut payload = serde_json::json!({ "appId": app_id, "mode": "app" });

    if let Some(id) = server_id {
        payload["serverId"] = serde_json::json!(id);
    }

    let answer = state
        .api
        .request(
            tmc_core::api::Method::POST,
            "/play/launch",
            Some(payload),
            false,
        )
        .await
        .ok()?;

    answer
        .get("authCode")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

/// The one folder TMC games keep downloaded content in. Fixed, and inside the
/// app's own data, because "clear" deletes it recursively.
fn game_cache_dir(state: &AppState) -> std::path::PathBuf {
    state.paths.data.join("game-cache")
}

fn dir_bytes(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };

    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            // Not followed: a link inside the cache is not the cache's to count.
            Ok(t) if t.is_dir() => dir_bytes(&entry.path()),
            Ok(t) if t.is_file() => entry.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

/// How much the kept game downloads take up.
#[tauri::command]
pub fn games_cache_size(state: State<'_, AppState>) -> AppResult<u64> {
    Ok(dir_bytes(&game_cache_dir(&state)))
}

/// Delete every kept game download; returns the bytes freed.
///
/// Refused while a game is running: it has packs from this folder mounted, and
/// deleting them under it is a crash on the next load rather than a smaller disk.
#[tauri::command]
pub fn games_cache_clear(state: State<'_, AppState>) -> AppResult<u64> {
    if !state.sessions.running().is_empty() {
        return Err(AppError::invalid(
            "Close the running game first; it is using these files.",
        ));
    }

    let dir = game_cache_dir(&state);
    let freed = dir_bytes(&dir);

    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(AppError::internal(format!(
                "could not clear the game cache: {e}"
            )))
        }
    }

    audit!(
        state.audit,
        Info,
        App,
        "games.cache.clear",
        "Game downloads cleared"
    );

    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_runs_on_the_engine_the_catalogue_names() {
        let answer = serde_json::json!({ "apps": [{
            "id": 114, "slug": "Arena", "name": "Arena",
            "engine": { "id": 84, "name": "Godot", "url": "godot" },
        }]});

        assert_eq!(
            engine_of(&answer, 114),
            Some(EngineGame {
                engine_id: 84,
                slug: Some("arena".into()),
                name: Some("Arena".into()),
            })
        );
    }

    #[test]
    fn no_engine_another_app_or_the_engine_itself_is_no_fallback() {
        let engine = serde_json::json!({ "apps": [{
            "id": 84, "slug": "godot", "name": "Godot", "engine": null,
        }]});
        let selfish = serde_json::json!({ "apps": [{
            "id": 84, "slug": "godot", "engine": { "id": 84 },
        }]});
        let other = serde_json::json!({ "apps": [{
            "id": 7, "slug": "rust", "engine": { "id": 86 },
        }]});

        assert_eq!(engine_of(&engine, 84), None);
        assert_eq!(engine_of(&selfish, 84), None);
        assert_eq!(engine_of(&other, 114), None);
        assert_eq!(engine_of(&serde_json::json!({}), 114), None);
    }

    /// A site that answers the two questions from fixed values.
    struct FakeSite {
        app: AppResult<serde_json::Value>,
        boot: AppResult<serde_json::Value>,
        asked_boot: std::cell::Cell<bool>,
    }

    fn copy(r: &AppResult<serde_json::Value>) -> AppResult<serde_json::Value> {
        match r {
            Ok(v) => Ok(v.clone()),
            Err(AppError::Network(m)) => Err(AppError::Network(m.clone())),
            Err(AppError::Api(m)) => Err(AppError::Api(m.clone())),
            Err(e) => Err(AppError::internal(e.to_string())),
        }
    }

    impl Catalogue for FakeSite {
        async fn app(&self, _: i64) -> AppResult<serde_json::Value> {
            copy(&self.app)
        }

        async fn solo_boot(&self, _: i64, _: Option<&str>) -> AppResult<serde_json::Value> {
            self.asked_boot.set(true);
            copy(&self.boot)
        }
    }

    fn arena_on_godot() -> serde_json::Value {
        serde_json::json!({ "apps": [{
            "id": 114, "slug": "arena", "name": "Arena", "engine": { "id": 84 },
        }]})
    }

    fn with_pack() -> serde_json::Value {
        serde_json::json!({ "boot": {
            "solo": { "contentId": "gamemann/game-arena", "version": "0.1.16" },
            "game": { "origin": "https://dotgames.org" },
        }})
    }

    fn site(app: AppResult<serde_json::Value>, boot: AppResult<serde_json::Value>) -> FakeSite {
        FakeSite {
            app,
            boot,
            asked_boot: std::cell::Cell::new(false),
        }
    }

    fn row(app_id: i64) -> InstalledGame {
        InstalledGame {
            app_id,
            slug: Some("godot".into()),
            name: "Godot".into(),
            platform: "LINUX_X64".into(),
            version: "1.0".into(),
            dir: "/games/godot".into(),
            entry: None,
            args: vec!["--".into()],
            size_bytes: 0,
            installed_ms: 0,
            updated_ms: 0,
            auto_update: false,
        }
    }

    /// Rows installed on this "machine".
    fn only(ids: &'static [i64]) -> impl Fn(i64) -> AppResult<Option<InstalledGame>> {
        move |id| Ok(ids.contains(&id).then(|| row(id)))
    }

    #[tokio::test]
    async fn a_pack_game_runs_on_its_installed_engine() {
        let s = site(Ok(arena_on_godot()), Ok(with_pack()));

        match native_target(&s, only(&[84]), 114, None).await {
            Ok(NativeTarget::Engine { row, game }) => {
                assert_eq!(row.app_id, 84);
                assert_eq!(game.slug.as_deref(), Some("arena"));
                assert_eq!(game.name.as_deref(), Some("Arena"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_game_with_its_own_build_never_asks_the_site() {
        let s = site(
            Err(AppError::Network("down".into())),
            Err(AppError::Network("down".into())),
        );

        match native_target(&s, only(&[114, 84]), 114, None).await {
            Ok(NativeTarget::Own(row)) => assert_eq!(row.app_id, 114),
            other => panic!("{other:?}"),
        }
        assert!(!s.asked_boot.get());
    }

    #[tokio::test]
    async fn a_pack_game_without_its_engine_installed_says_so() {
        let s = site(Ok(arena_on_godot()), Ok(with_pack()));

        match native_target(&s, only(&[]), 114, None).await {
            Ok(NativeTarget::EngineMissing { engine_id }) => assert_eq!(engine_id, 84),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn another_game_on_the_same_engine_does_not_fall_back() {
        // A third-party game made with the engine: the site boots it with no
        // pack, refuses it, or has nothing for it.
        for boot in [
            Ok(serde_json::json!({ "boot": { "game": { "origin": "https://dotgames.org" } } })),
            Ok(serde_json::json!({ "boot": { "solo": null } })),
            Ok(serde_json::Value::Null),
            Err(AppError::Api("refused".into())),
        ] {
            let s = site(Ok(arena_on_godot()), boot);

            match native_target(&s, only(&[84]), 114, None).await {
                Ok(NativeTarget::Nothing) => {}
                other => panic!("{other:?}"),
            }
            assert!(s.asked_boot.get());
        }
    }

    #[tokio::test]
    async fn an_app_with_no_engine_is_not_installed() {
        let s = site(
            Ok(serde_json::json!({ "apps": [{ "id": 114, "engine": null }] })),
            Ok(with_pack()),
        );

        match native_target(&s, only(&[84]), 114, None).await {
            Ok(NativeTarget::Nothing) => {}
            other => panic!("{other:?}"),
        }
        assert!(!s.asked_boot.get());
    }

    #[tokio::test]
    async fn an_unreachable_catalogue_is_an_error_not_not_installed() {
        for s in [
            site(Err(AppError::Network("down".into())), Ok(with_pack())),
            site(Err(AppError::internal("http: 502")), Ok(with_pack())),
            site(Ok(arena_on_godot()), Err(AppError::Network("down".into()))),
        ] {
            match native_target(&s, only(&[84]), 114, None).await {
                Err(AppError::Network(m)) => assert_eq!(m, CATALOGUE_UNREACHABLE),
                other => panic!("{other:?}"),
            }
        }
    }
}
