use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, StatusCode};

// Re-exported so callers name the verb through this module rather than taking
// a direct dependency on reqwest.
pub use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::auth::AuthState;
use crate::error::{AppError, AppResult};
use crate::secure::SecureStore;

/// The only thing in the app that talks to The Modding Community.
///
/// Everything about it exists to keep one property true: **the webview never
/// holds a credential.** The frontend calls `api_get`/`api_post` with a path
/// and gets parsed JSON back; the `Authorization` header is added here, the
/// refresh dance happens here, and a 401 is resolved here rather than being
/// handed to React to figure out.
///
/// The base URL is fixed at compile time. It is not a setting and not a
/// parameter from the webview, because a "which server?" field is a phishing
/// primitive: point the app at a look-alike host and it will happily send that
/// host a bearer token. `TMC_API_BASE` exists only so a dev build can target a
/// local instance, and it is read from the build environment, not at runtime.
pub const API_BASE: &str = match option_env!("TMC_API_BASE") {
    Some(v) => v,
    None => "https://moddingcommunity.com",
};

/// Refresh-token reuse detection means two concurrent refreshes would sign the
/// user out. This lock makes the second caller wait for the first's result.
pub struct ApiClient {
    http: Client,
    auth: Arc<AuthState>,
    secure: Arc<SecureStore>,
    refresh_lock: Mutex<()>,
}

#[derive(Deserialize)]
struct Envelope {
    ok: bool,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

impl ApiClient {
    pub fn new(auth: Arc<AuthState>, secure: Arc<SecureStore>, version: &str) -> AppResult<Self> {
        let http = Client::builder()
            .user_agent(format!("TMC-App/{version}"))
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            // The API is ours and speaks HTTPS. A redirect chain off it is
            // either a misconfiguration or an attack, and following one would
            // resend the bearer to wherever it points.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::internal(format!("http client: {e}")))?;

        Ok(Self {
            http,
            auth,
            secure,
            refresh_lock: Mutex::new(()),
        })
    }

    fn url(path: &str) -> AppResult<String> {
        /*
         * The path comes from the webview, so it is checked rather than
         * trusted. Anything absolute, protocol-relative, or containing a `..`
         * segment is refused — those are the three shapes that turn "call our
         * API" into "call whatever host the caller names".
         *
         * Only the PATH is checked, not the query string. Checking the whole
         * thing looks safer and is actively wrong: `..` is perfectly legal in a
         * query value, so `?search=Foo..Bar` — or a user simply typing two full
         * stops into the search box — was refused before the request was made.
         * A query value cannot climb a path; only a path segment can.
         */
        let (route, query) = match path.split_once('?') {
            Some((route, query)) => (route, Some(query)),
            None => (path, None),
        };

        if !route.starts_with('/') || route.starts_with("//") || route.contains("..") {
            return Err(AppError::invalid("Bad API path."));
        }

        // A `#` would make everything after it a fragment the server never
        // sees, which is a way to hide part of a path from this check.
        if route.contains('#') || query.is_some_and(|q| q.contains('#')) {
            return Err(AppError::invalid("Bad API path."));
        }

        Ok(format!("{API_BASE}/api/app/v1{path}"))
    }

    /// A request with the bearer attached, retried once through a refresh.
    ///
    /// One retry, never a loop: if a freshly minted token is also rejected the
    /// problem is not staleness, and retrying would turn a server-side fault
    /// into an infinite request storm from every installed copy of the app.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        require_auth: bool,
    ) -> AppResult<Value> {
        let url = Self::url(path)?;

        let mut token = self.access_token(require_auth).await?;

        for attempt in 0..2 {
            let mut req = self.http.request(method.clone(), &url);

            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }

            if let Some(b) = &body {
                req = req.json(b);
            }

            let res = req.send().await?;
            let status = res.status();
            let text = res.text().await.unwrap_or_default();

            // A 401 on the first attempt is the expected way to discover a
            // token that expired between calls.
            if status == StatusCode::UNAUTHORIZED && attempt == 0 && self.auth.has_refresh() {
                token = Some(self.refresh().await?);
                continue;
            }

            return Self::unwrap(status, &text);
        }

        Err(AppError::NotAuthenticated)
    }

    /// Envelope → value, or the API's own error turned into an [`AppError`].
    fn unwrap(status: StatusCode, text: &str) -> AppResult<Value> {
        let envelope: Envelope = serde_json::from_str(text).map_err(|_| {
            // A non-JSON body from our own API means something in front of it
            // answered — a proxy error page, a captive portal.
            AppError::Network(format!("Unexpected response from the server ({status})."))
        })?;

        if envelope.ok {
            return Ok(envelope.data);
        }

        let code = envelope.code.unwrap_or_else(|| "api".into());
        let message = envelope
            .message
            .unwrap_or_else(|| "The request failed.".into());

        Err(match code.as_str() {
            "unauthorized" | "token_expired" => AppError::NotAuthenticated,
            "token_reuse" | "invalid_grant" => AppError::AuthRejected,
            _ => AppError::Api(message),
        })
    }

    async fn access_token(&self, require_auth: bool) -> AppResult<Option<String>> {
        if let Some(token) = self.auth.access_token() {
            return Ok(Some(token));
        }

        if !self.auth.has_refresh() {
            return if require_auth {
                Err(AppError::NotAuthenticated)
            } else {
                Ok(None)
            };
        }

        Ok(Some(self.refresh().await?))
    }

    /// Exchange the refresh token for a new pair.
    ///
    /// Serialised by `refresh_lock`, and the lock is re-checked afterwards: the
    /// caller that lost the race must use the winner's token rather than
    /// spending its own now-retired refresh token, which the server would read
    /// as reuse and answer by revoking every device.
    async fn refresh(&self) -> AppResult<String> {
        let _guard = self.refresh_lock.lock().await;

        if let Some(token) = self.auth.access_token() {
            return Ok(token);
        }

        let refresh = self
            .auth
            .refresh_token()
            .ok_or(AppError::NotAuthenticated)?;

        let res = self
            .http
            .post(format!("{API_BASE}/api/app/v1/auth/refresh"))
            .json(&serde_json::json!({ "refreshToken": refresh }))
            .send()
            .await?;

        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        let data = match Self::unwrap(status, &text) {
            Ok(v) => v,
            Err(e) => {
                // A refused refresh is terminal: the stored token is dead and
                // keeping it would mean retrying it on every request.
                if matches!(e, AppError::AuthRejected | AppError::NotAuthenticated) {
                    self.auth.sign_out(&self.secure);
                }

                return Err(e);
            }
        };

        let access = data
            .get("accessToken")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::internal("refresh: no accessToken"))?
            .to_string();

        let next_refresh = data
            .get("refreshToken")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::internal("refresh: no refreshToken"))?
            .to_string();

        let expires_in = data
            .get("expiresIn")
            .and_then(Value::as_u64)
            .unwrap_or(3600);

        if let Some(user) = data.get("user") {
            if let Ok(parsed) = serde_json::from_value(user.clone()) {
                self.auth.set_user(Some(parsed));
            }
        }

        self.auth
            .store_tokens(&self.secure, access.clone(), next_refresh, expires_in)?;

        Ok(access)
    }

    /// An unauthenticated call — the device-grant endpoints, which by
    /// definition happen before there is anything to authenticate with.
    pub async fn anonymous_post(&self, path: &str, body: Value) -> AppResult<Value> {
        let url = Self::url(path)?;

        let res = self.http.post(url).json(&body).send().await?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();

        Self::unwrap(status, &text)
    }

    /// The raw envelope for the device poll, whose non-success outcomes
    /// (`authorization_pending`) are normal states rather than failures and so
    /// must not be flattened into an [`AppError`].
    pub async fn poll_device(&self, body: Value) -> AppResult<(bool, String, Value)> {
        let url = Self::url("/auth/token")?;

        let res = self.http.post(url).json(&body).send().await?;
        let text = res.text().await.unwrap_or_default();

        let envelope: Envelope = serde_json::from_str(&text)
            .map_err(|_| AppError::Network("Unexpected response from the server.".into()))?;

        Ok((
            envelope.ok,
            envelope.code.unwrap_or_default(),
            envelope.data,
        ))
    }

    /// Shared HTTP client for plugin downloads, so they inherit the same
    /// timeouts and TLS stack. Never carries the bearer — see
    /// [`crate::plugins::steps`], which builds its own request.
    pub fn raw(&self) -> &Client {
        &self.http
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_that_could_name_another_host_are_refused() {
        for bad in [
            "no-leading-slash",
            "//evil.test/x",
            "/../../admin",
            "/browse/..%2f..",
            "/browse#/../x",
            "/browse?q=x#y",
            "",
        ] {
            assert!(ApiClient::url(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn dots_in_a_query_value_are_allowed() {
        // The regression this test exists for: two full stops typed into the
        // search box used to fail before the request left the app.
        for good in [
            "/browse?kind=mod&search=Foo..Bar",
            "/browse?kind=mod&search=..",
            "/browse?kind=mod&cursor=12345",
            "/me",
        ] {
            assert!(ApiClient::url(good).is_ok(), "{good} should be allowed");
        }
    }

    #[test]
    fn the_base_is_always_our_own_api() {
        let url = ApiClient::url("/browse?kind=mod").expect("valid");

        assert!(url.starts_with(API_BASE));
        assert!(url.contains("/api/app/v1/browse"));
    }
}
