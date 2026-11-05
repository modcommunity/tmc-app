use serde::Serialize;
use serde_json::Value;
use tauri::State;

use tmc_core::api::Method;
use tmc_core::audit;
use tmc_core::error::{AppError, AppResult};
/*
 * Whether one version is later than another, for the update check below.
 *
 * It used to be a private function in this file, and then the game installer
 * needed the same question answered about a build on disk. A second copy would
 * have been a second place for `1.10.0` to sort below `1.9.0` — the one mistake
 * in this app that looks right until somebody releases a tenth minor version.
 */
use tmc_core::version::is_newer;

use crate::state::AppState;

/// A GET against `/api/app/v1<path>`.
///
/// `path` is validated inside `ApiClient`; there is no way to name a different
/// host. `query` is serialised here rather than concatenated by the caller, so
/// a value containing `&` cannot inject a parameter.
#[tauri::command]
pub async fn api_get(
    state: State<'_, AppState>,
    path: String,
    query: Option<Vec<(String, String)>>,
    auth: Option<bool>,
) -> AppResult<Value> {
    let full = match query {
        Some(pairs) if !pairs.is_empty() => {
            let encoded = pairs
                .iter()
                .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
                .collect::<Vec<_>>()
                .join("&");

            format!("{path}?{encoded}")
        }
        _ => path,
    };

    state
        .api
        .request(Method::GET, &full, None, auth.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn api_send(
    state: State<'_, AppState>,
    method: String,
    path: String,
    body: Option<Value>,
    // Defaults to true. False means the route answers a signed-out caller
    // (auth optional on the site): the bearer is still attached when there is
    // one, exactly as api_get does, so this only stops a missing sign-in from
    // being refused before the request leaves.
    auth: Option<bool>,
) -> AppResult<Value> {
    let method = match method.to_ascii_uppercase().as_str() {
        "POST" => Method::POST,
        "PATCH" => Method::PATCH,
        "PUT" => Method::PUT,
        "DELETE" => Method::DELETE,
        // Only the verbs the app actually uses. An open method parameter is a
        // request-smuggling primitive against anything in front of the API.
        other => return Err(AppError::invalid(format!("Method {other} is not allowed."))),
    };

    state
        .api
        .request(method, &path, body, auth.unwrap_or(true))
        .await
}

/// Which site this build talks to, and whether that is the real one.
///
/// Read-only and carries no credential — the base is not a secret, and the
/// webview cannot set it: there is no matching `api_set_base`, and there must
/// never be one. It exists so the app can SAY on screen that it is pointed at a
/// dev instance, which is the difference between "this build is talking to
/// staging" and a bug report about data that was never there.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiEnv {
    /// Origin only — `https://tmcdev.net:3002`, never a path.
    pub base: String,
    pub is_prod: bool,
    pub version: String,
}

#[tauri::command]
pub fn api_env(state: State<'_, AppState>) -> ApiEnv {
    ApiEnv {
        base: tmc_core::api::api_base().to_string(),
        is_prod: tmc_core::api::api_base_is_prod(),
        version: state.version.clone(),
    }
}

/// Percent-encode everything outside the unreserved set.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_characters_are_encoded() {
        assert_eq!(urlencode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(urlencode("safe-_.~"), "safe-_.~");
        // Multi-byte input encodes per byte, which is what a URL wants.
        assert_eq!(urlencode("é"), "%C3%A9");
    }
}

// ------------------------------------------------------------------ Updates

use tmc_core::updater::{Compiled, UpdaterSource};

/// What an update check found.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// This build.
    pub current: String,
    /// The newest version found, or `None` when nothing says anything.
    pub latest: Option<String>,
    /// Where to get it in a browser. Absolute https, validated here.
    pub download: Option<String>,
    /// Whether `latest` is actually ahead of `current`.
    pub outdated: bool,
    /// Whether THIS build can install the update itself.
    ///
    /// False on mobile, where an app replacing its own binary is not a thing
    /// the OS permits, and false when no signing key is configured anywhere —
    /// see [`tmc_core::updater`]. Published so the banner offers the button it
    /// can actually honour: "Update" that turns out to open a browser is worse
    /// than "Download" that says what it does.
    pub installable: bool,
}

/// The compiled-in half of the updater's inputs.
///
/// `TMC_UPDATER_PUBKEY` is filled by `build.rs` from `updater.pub` when the
/// environment does not set it; `TMC_UPDATER_ENDPOINT` is the public
/// `latest.json` the release workflow uploads (`downloads/tmc-app/latest.json`
/// under the S3 bucket's public URL) and is unset in a developer build, which
/// then asks the site.
fn compiled() -> Compiled<'static> {
    Compiled {
        pubkey: option_env!("TMC_UPDATER_PUBKEY"),
        endpoint: option_env!("TMC_UPDATER_ENDPOINT"),
    }
}

/// Where this check goes and which key it trusts, right now.
fn updater_source(state: &AppState) -> UpdaterSource {
    let settings = state.settings.get();

    tmc_core::updater::resolve(
        settings.updater_endpoint.as_deref(),
        settings.updater_pubkey.as_deref(),
        &settings.update_channel,
        compiled(),
        tmc_core::api::api_base(),
    )
}

/// Whether this build, as configured, can install an update itself.
pub fn updater_available(state: &AppState) -> bool {
    cfg!(desktop) && updater_source(state).pubkey.is_some()
}

/// Ask whether this build is out of date.
///
/// **This does not update anything.** Two sources, in order:
///
///   1. With a key configured on desktop, the updater's own endpoint — by
///      default the static `latest.json` on the downloads bucket, which works
///      with nothing configured on the site. `check()` only fetches and parses
///      the manifest; nothing is downloaded and no signature is needed yet.
///   2. Otherwise, or when that fails, the site's `/version` — the honest
///      floor every build has, including mobile and a build with no key, which
///      is answered with a download page to open in a browser.
///
/// Unauthenticated, because a freshly-installed app that has not signed in yet
/// is precisely the one most likely to be out of date.
#[tauri::command]
pub async fn update_check(
    #[allow(unused_variables)] app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<UpdateCheck> {
    let current = state.version.clone();
    let installable = updater_available(&state);

    #[cfg(desktop)]
    if installable {
        match updater_manifest(&app, &state).await {
            Ok(latest) => {
                let outdated = latest
                    .as_deref()
                    .is_some_and(|latest| is_newer(latest, &current));

                if outdated {
                    audit_available(&state, &current, latest.as_deref());
                }

                return Ok(UpdateCheck {
                    current,
                    latest,
                    download: None,
                    outdated,
                    installable,
                });
            }
            Err(e) => {
                // Fall through to the site: an unreachable bucket must not
                // hide an update the site knows about.
                tracing::warn!("updater manifest check failed: {}", e.detail());
            }
        }
    }

    let body = state
        .api
        .request(tmc_core::api::Method::GET, "/version", None, false)
        .await?;

    let latest = body
        .get("latest")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string);

    let download = body
        .get("download")
        .and_then(serde_json::Value::as_str)
        .filter(|u| u.starts_with("https://"))
        .map(str::to_string);

    let outdated = latest
        .as_deref()
        .is_some_and(|latest| is_newer(latest, &current));

    if outdated {
        audit_available(&state, &current, latest.as_deref());
    }

    Ok(UpdateCheck {
        current,
        latest,
        download,
        outdated,
        installable,
    })
}

fn audit_available(state: &AppState, current: &str, latest: Option<&str>) {
    audit!(
        state.audit,
        Info,
        App,
        "app.update.available",
        format!("{current} → {}", latest.unwrap_or("?"))
    );
}

/// An updater built from the resolved source, with its key set per call.
///
/// Refuses when there is no key, which is the "no updater" state — the plugin
/// is registered either way, so this is the one gate.
#[cfg(desktop)]
fn build_updater(
    app: &tauri::AppHandle,
    source: &UpdaterSource,
) -> AppResult<tauri_plugin_updater::Updater> {
    use tauri_plugin_updater::UpdaterExt;

    let Some(pubkey) = source.pubkey.as_deref() else {
        return Err(AppError::invalid(
            "This build has no update signing key configured, so it cannot install updates. Use the download page, or have an administrator set `updaterPubkey` in settings.json.",
        ));
    };

    let endpoint = source
        .endpoint
        .parse()
        .map_err(|_| AppError::internal("the update endpoint did not parse"))?;

    app.updater_builder()
        .pubkey(pubkey)
        .endpoints(vec![endpoint])
        .map_err(|e| AppError::internal(format!("updater: {e}")))?
        .build()
        .map_err(|e| AppError::internal(format!("updater: {e}")))
}

/// The version the updater's manifest offers, without downloading anything.
#[cfg(desktop)]
async fn updater_manifest(app: &tauri::AppHandle, state: &AppState) -> AppResult<Option<String>> {
    let updater = build_updater(app, &updater_source(state))?;

    let found = updater
        .check()
        .await
        .map_err(|e| AppError::internal(format!("update check: {e}")))?;

    Ok(found.map(|u| u.version))
}

/// Download the update, verify its signature, and install it.
///
/// **The signature is the only thing making this safe**, and it is checked by
/// the plugin against the resolved key — not by the server, not by TLS. TLS
/// says who served the bytes; it says nothing about what they are, and this
/// call replaces the program the user is running.
///
/// Three refusals, all of them before anything is fetched:
///
///   * a build with no key anywhere has no updater and says so;
///   * a check that finds nothing newer does nothing, rather than reinstalling
///     the version already running;
///   * a running game blocks it. Restarting the app out from under a supervisor
///     thread that is holding a `Child` ends the play session with no duration
///     and loses whatever the game had printed.
///
/// It does not relaunch. Tauri's installers take over on Windows and macOS, and
/// deciding for somebody that now is the moment to close their app is not this
/// command's call — the UI says the update is ready and they restart when they
/// are done.
#[cfg(desktop)]
#[tauri::command]
pub async fn update_install(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> AppResult<UpdateCheck> {
    if !state.sessions.running().is_empty() {
        return Err(AppError::invalid(
            "A game is running. Close it before updating the app.",
        ));
    }

    let source = updater_source(&state);
    let updater = build_updater(&app, &source)?;

    let found = updater
        .check()
        .await
        .map_err(|e| AppError::internal(format!("update check: {e}")))?;

    let current = state.version.clone();

    let Some(update) = found else {
        return Ok(UpdateCheck {
            current,
            latest: None,
            download: None,
            outdated: false,
            installable: true,
        });
    };

    let version = update.version.clone();

    audit!(
        state.audit,
        Security,
        App,
        "app.update.install",
        format!(
            "{current} → {version} from {} (key: {:?})",
            source.endpoint, source.key_origin
        )
    );

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| AppError::internal(format!("update install: {e}")))?;

    Ok(UpdateCheck {
        current,
        latest: Some(version),
        download: None,
        outdated: true,
        installable: true,
    })
}

/// What Settings → App → Updates shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterStatus {
    /// Desktop, and a key resolved from somewhere.
    pub available: bool,
    /// False on mobile, where there is no self-updater whatever is configured.
    pub supported: bool,
    #[serde(flatten)]
    pub source: UpdaterSource,
    /// Whether a key was compiled into this binary, so the screen can say
    /// "clear the override to go back to the built-in key" truthfully.
    pub compiled_key: bool,
    /// The endpoint this build falls back to with no override.
    pub compiled_endpoint: String,
    /// The overrides as stored, for the form to show.
    pub endpoint_override: Option<String>,
    pub pubkey_override: Option<String>,
}

#[tauri::command]
pub fn updater_status(state: State<'_, AppState>) -> UpdaterStatus {
    let settings = state.settings.get();
    let source = updater_source(&state);
    let base =
        tmc_core::updater::resolve(None, None, "stable", compiled(), tmc_core::api::api_base());

    UpdaterStatus {
        available: cfg!(desktop) && source.pubkey.is_some(),
        supported: cfg!(desktop),
        source,
        compiled_key: base.pubkey.is_some(),
        compiled_endpoint: base.endpoint,
        endpoint_override: settings.updater_endpoint,
        pubkey_override: settings.updater_pubkey,
    }
}

/// The link the app was started with, once; the webview calls this on mount.
#[tauri::command]
pub fn deep_link_take(app: tauri::AppHandle) -> Option<tmc_core::deeplink::DeepLink> {
    use tauri::Manager;

    app.try_state::<tmc_core::deeplink::PendingLink>()?.take()
}
