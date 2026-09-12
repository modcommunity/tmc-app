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

    state.api.request(method, &path, body, true).await
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

/// What an update check found.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// This build.
    pub current: String,
    /// The newest the site knows about, or `None` when it says nothing.
    pub latest: Option<String>,
    /// Where to get it. Absolute https, already validated server-side.
    pub download: Option<String>,
    /// Whether `latest` is actually ahead of `current`.
    pub outdated: bool,
    /// Whether THIS build can install the update itself.
    ///
    /// False for a build compiled without `TMC_UPDATER_PUBKEY` and false on
    /// mobile, where an app replacing its own binary is not a thing the OS
    /// permits. Published so the banner offers the button it can actually
    /// honour: "Update" that turns out to open a browser is worse than
    /// "Download" that says what it does.
    pub installable: bool,
}

/// Ask the site whether this build is out of date.
///
/// **This does not update anything.** The shape of the answer says so: no
/// artifact, no signature, no checksum. A user who is behind is offered a link
/// to the download page, which opens in their real browser.
///
/// It is deliberately not [`update_install`] with a different name. This is the
/// answer every build can give — including one compiled without a signing key,
/// which has no updater at all — so it stays the honest floor and the fallback
/// for a platform the updater does not cover. `UpdateCheck::installable` is
/// what tells the UI which of the two it is looking at.
///
/// Unauthenticated, because a freshly-installed app that has not signed in yet
/// is precisely the one most likely to be out of date.
#[tauri::command]
pub async fn update_check(state: State<'_, AppState>) -> AppResult<UpdateCheck> {
    let current = state.version.clone();

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
        audit!(
            state.audit,
            Info,
            App,
            "app.update.available",
            format!("{current} → {}", latest.as_deref().unwrap_or("?"))
        );
    }

    Ok(UpdateCheck {
        current,
        latest,
        download,
        outdated,
        installable: updater_available(),
    })
}

/// Whether this build carries a working updater.
///
/// Desktop, and a signing public key compiled in. The key is the whole security
/// model — the plugin verifies a minisign signature before it installs anything
/// — so a build without one is a build that cannot tell a real update from an
/// attacker's, and the correct behaviour for it is to have no updater rather
/// than a trusting one. See `lib.rs` for why there is no placeholder key.
pub fn updater_available() -> bool {
    cfg!(desktop)
        && option_env!("TMC_UPDATER_PUBKEY")
            .map(str::trim)
            .is_some_and(|key| !key.is_empty())
}

/// Download the update, verify its signature, and install it.
///
/// **The signature is the only thing making this safe**, and it is checked by
/// the plugin against the key compiled into this binary — not by the server,
/// not by TLS. TLS says who served the bytes; it says nothing about what they
/// are, and this call replaces the program the user is running.
///
/// Three refusals, all of them before anything is fetched:
///
///   * a build with no key compiled in has no updater and says so;
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
    use tauri_plugin_updater::UpdaterExt;

    if !updater_available() {
        return Err(AppError::invalid(
            "This build cannot install updates itself. Use the download page.",
        ));
    }

    if !state.sessions.running().is_empty() {
        return Err(AppError::invalid(
            "A game is running. Close it before updating the app.",
        ));
    }

    /*
     * Endpoints are set HERE rather than in `tauri.conf.json`, because the
     * config is static and the base is not: a build pointed at the dev site has
     * to ask the dev site. It is the same rule every other URL in this app
     * follows, and `api_base()` has already validated the scheme and host.
     */
    let endpoint = format!(
        "{}/api/app/v1/update/{{{{target}}}}/{{{{arch}}}}/{{{{current_version}}}}",
        tmc_core::api::api_base().trim_end_matches('/')
    )
    .parse()
    .map_err(|_| AppError::internal("the update endpoint did not parse"))?;

    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| AppError::internal(format!("updater: {e}")))?
        .build()
        .map_err(|e| AppError::internal(format!("updater: {e}")))?;

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
        format!("{current} → {version}")
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
