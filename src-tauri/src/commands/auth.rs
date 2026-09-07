use serde::Serialize;
use serde_json::Value;
use tauri::State;

use tmc_core::api::Method;
use tmc_core::audit;
use tmc_core::auth::{client_info, PendingLogin, SessionUser};
use tmc_core::error::{AppError, AppResult};

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub signed_in: bool,
    pub pending: bool,
    pub user: Option<SessionUser>,
}

#[tauri::command]
pub fn session_state(state: State<'_, AppState>) -> SessionSnapshot {
    SessionSnapshot {
        signed_in: state.auth.has_refresh(),
        pending: state.auth.is_pending(),
        user: state.auth.user(),
    }
}

/// Start a device login and hand back what to show the user.
///
/// The PKCE verifier is generated and kept in Rust. The webview receives only
/// the user code and the URL to open, neither of which is a secret — the user
/// code is meant to be read aloud.
#[tauri::command]
pub async fn auth_begin(state: State<'_, AppState>) -> AppResult<PendingLogin> {
    let (verifier, challenge) = state.auth.begin()?;

    let body = serde_json::json!({
        "client": client_info(&state.version),
        "codeChallenge": challenge,
    });

    let data = state.api.anonymous_post("/auth/device", body).await?;

    let get = |key: &str| -> AppResult<String> {
        data.get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AppError::internal(format!("device start: missing {key}")))
    };

    let device_code = get("deviceCode")?;
    let expires_in = data.get("expiresIn").and_then(Value::as_u64).unwrap_or(600);

    state.auth.arm(device_code, verifier, expires_in);

    audit!(
        state.audit,
        Security,
        Auth,
        "auth.device.start",
        "Started a sign-in and opened the browser"
    );

    Ok(PendingLogin {
        user_code: get("userCode")?,
        verification_uri: get("verificationUri")?,
        verification_uri_complete: get("verificationUriComplete")?,
        expires_in,
        interval: data.get("interval").and_then(Value::as_u64).unwrap_or(5),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollOutcome {
    /// `pending`, `signedIn`, `denied`, `expired`.
    pub status: &'static str,
    pub user: Option<SessionUser>,
}

/// One poll of the in-flight login.
///
/// Called on a timer by the frontend AND on a deep-link wake-up, which is why
/// the server side claims the grant atomically — both can land at once.
#[tauri::command]
pub async fn auth_poll(state: State<'_, AppState>) -> AppResult<PollOutcome> {
    let Some((device_code, verifier)) = state.auth.poll_pair() else {
        return Ok(PollOutcome {
            status: "expired",
            user: None,
        });
    };

    let body = serde_json::json!({
        "deviceCode": device_code,
        "codeVerifier": verifier,
    });

    let (ok, code, data) = state.api.poll_device(body).await?;

    if !ok {
        return Ok(match code.as_str() {
            "authorization_pending" | "slow_down" => PollOutcome {
                status: "pending",
                user: None,
            },
            "access_denied" => {
                state.auth.clear_pending();

                audit!(
                    state.audit,
                    Security,
                    Auth,
                    "auth.device.denied",
                    "Sign-in was rejected in the browser"
                );

                PollOutcome {
                    status: "denied",
                    user: None,
                }
            }
            _ => {
                state.auth.clear_pending();

                PollOutcome {
                    status: "expired",
                    user: None,
                }
            }
        });
    }

    let access = data
        .get("accessToken")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::internal("device token: no accessToken"))?;

    let refresh = data
        .get("refreshToken")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::internal("device token: no refreshToken"))?;

    let expires_in = data
        .get("expiresIn")
        .and_then(Value::as_u64)
        .unwrap_or(3600);

    let user: Option<SessionUser> = data
        .get("user")
        .and_then(|u| serde_json::from_value(u.clone()).ok());

    state.auth.store_tokens(
        &state.secure,
        access.to_string(),
        refresh.to_string(),
        expires_in,
    )?;

    state.auth.set_user(user.clone());
    state.auth.clear_pending();

    audit!(
        state.audit,
        Security,
        Auth,
        "auth.device.complete",
        format!(
            "Signed in as {}",
            user.as_ref()
                .and_then(|u| u.username.clone().or_else(|| u.name.clone()))
                .unwrap_or_else(|| "unknown".into())
        )
    );

    Ok(PollOutcome {
        status: "signedIn",
        user,
    })
}

#[tauri::command]
pub async fn auth_sign_out(state: State<'_, AppState>) -> AppResult<()> {
    /*
     * Best effort. The local credentials are dropped either way — a network
     * failure must not leave the user still signed in on their own device.
     * An empty OBJECT, not `null`: the route's schema is `z.object({})`, and a
     * JSON null would be rejected before the revocation ran.
     */
    let _ = state
        .api
        .request(
            Method::POST,
            "/auth/revoke",
            Some(serde_json::json!({})),
            true,
        )
        .await;

    state.auth.sign_out(&state.secure);

    audit!(state.audit, Security, Auth, "auth.signout", "Signed out");

    Ok(())
}

#[tauri::command]
pub fn auth_cancel(state: State<'_, AppState>) {
    state.auth.clear_pending();
}
