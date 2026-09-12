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
}

/// Ask the site whether this build is out of date.
///
/// **This does not update anything**, and the shape of the answer says so:
/// there is no artifact, no signature and no checksum in it. A user who is
/// behind is offered a link to the download page, which opens in their real
/// browser.
///
/// That is the whole feature, and it is deliberately not the other one. A
/// self-updater needs a signing key held by whoever cuts releases and a
/// manifest endpoint to serve; shipping the client half against neither is
/// exactly the "setting that names a capability it does not have" problem this
/// was written to fix, in a place where the consequence is a silently-installed
/// binary rather than an unchecked plugin.
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
    })
}
