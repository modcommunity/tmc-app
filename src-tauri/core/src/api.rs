use std::sync::{Arc, OnceLock};
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

/// The live site, and the base a shipped build talks to unless it was compiled
/// to talk to another one.
pub const PROD_API_BASE: &str = "https://moddingcommunity.com";

/// The base this binary was BUILT for: `TMC_API_BASE` from the build
/// environment, or production.
///
/// This is the only mechanism available to a release build, and it is the one
/// mobile uses — an Android or iOS process has no shell to inherit an
/// environment variable from, but the machine that compiles it does.
const BUILT_API_BASE: &str = match option_env!("TMC_API_BASE") {
    Some(v) => v,
    None => PROD_API_BASE,
};

/// Whether `TMC_API_BASE` is consulted at RUN time as well as at build time.
///
/// Debug builds only, which is precisely `npm run desktop` and not
/// `npm run desktop:build`. A shipped binary therefore has no code path that
/// reads a base from anywhere — the environment of the process that launched
/// the app is not a trust boundary (a `.desktop` file, a shortcut's "Start in",
/// an installer's launch step can all set one), and the value decides where a
/// bearer token is sent.
const RUNTIME_OVERRIDE: bool = cfg!(debug_assertions);

static RESOLVED_BASE: OnceLock<String> = OnceLock::new();

/// The base every request in this process goes to, resolved once.
///
/// Two ways to move it off production, in precedence order:
///
///   1. **`TMC_API_BASE` in the environment**, debug builds only. Survives a
///      rebuild, so `TMC_API_BASE=https://tmcdev.net:3002 npm run desktop`
///      needs no `cargo clean` and no recompile to go back.
///   2. **`TMC_API_BASE` at compile time**, any profile. Baked in; the only
///      option for Android, iOS and any packaged build.
///
/// An override that does not [`normalise_base`] is REFUSED and logged, and the
/// built-in base is used instead. Falling through to production on a typo is
/// the safe direction: the failure is "my dev server saw no traffic", not "my
/// dev credentials went to a host I fat-fingered".
pub fn api_base() -> &'static str {
    RESOLVED_BASE.get_or_init(|| {
        if !RUNTIME_OVERRIDE {
            return BUILT_API_BASE.to_string();
        }

        match std::env::var("TMC_API_BASE") {
            Ok(raw) if !raw.trim().is_empty() => match normalise_base(&raw) {
                Ok(base) => {
                    tracing::warn!("API base overridden by TMC_API_BASE: {base}");
                    base
                }
                Err(why) => {
                    tracing::error!("TMC_API_BASE ignored ({why}); using {BUILT_API_BASE}");
                    BUILT_API_BASE.to_string()
                }
            },
            _ => BUILT_API_BASE.to_string(),
        }
    })
}

/// Whether this process is pointed at the real site.
///
/// The frontend asks so it can say so on screen. A build talking to a staging
/// copy that looks identical to the real one is how a developer ends up filing
/// a bug about data that was never there.
pub fn api_base_is_prod() -> bool {
    api_base() == PROD_API_BASE
}

/// A tag that distinguishes a non-production base's stored state from
/// production's, or `None` on production.
///
/// [`crate::secure::SecureStore`] keys the refresh token on it. Without that,
/// pointing a signed-in app at a dev server sends the PRODUCTION refresh token
/// to that server on the first refresh — which leaks a live credential to a
/// box with dev-grade security, and then signs the user out of the real site
/// when it is rejected.
pub fn api_base_scope() -> Option<String> {
    if api_base_is_prod() {
        return None;
    }

    let tag: String = api_base()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    Some(tag)
}

/// An override string → the origin to use, or why it was refused.
///
/// Deliberately strict, because everything downstream concatenates onto it:
/// scheme and host only, no path, no query, no credentials. `Url::origin`
/// does the canonicalising, so `HTTPS://TMCDEV.NET:443/` and
/// `https://tmcdev.net` resolve to the same string rather than to two bases
/// that compare unequal.
///
/// `http` is allowed only for hosts that cannot be on the public internet. A
/// dev server on the LAN over plaintext is a normal thing to have; a plaintext
/// base pointing anywhere else means a bearer token crosses the network in the
/// clear, and no development convenience is worth teaching that.
fn normalise_base(raw: &str) -> Result<String, String> {
    let url = url::Url::parse(raw.trim()).map_err(|e| format!("not a URL: {e}"))?;

    match url.scheme() {
        "https" => {}
        "http" => {
            let host = url.host().ok_or_else(|| "no host".to_string())?;

            if !is_private_host(&host) {
                return Err(format!("http is only allowed for a local host, not {host}"));
            }
        }
        other => return Err(format!("scheme {other} is not http(s)")),
    }

    if url.host().is_none() {
        return Err("no host".into());
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err("credentials in the URL".into());
    }

    if url.path() != "/" && !url.path().is_empty() {
        return Err(format!("a path ({}) is not part of a base", url.path()));
    }

    if url.query().is_some() || url.fragment().is_some() {
        return Err("a query or fragment is not part of a base".into());
    }

    Ok(url.origin().ascii_serialization())
}

/// Hosts that cannot be reached from outside the machine or its LAN.
fn is_private_host(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Ipv4(ip) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        url::Host::Ipv6(ip) => ip.is_loopback() || ip.segments()[0] & 0xfe00 == 0xfc00,
        url::Host::Domain(name) => {
            let name = name.to_ascii_lowercase();

            name == "localhost"
                || name.ends_with(".localhost")
                || name.ends_with(".local")
                || name.ends_with(".test")
                || name.ends_with(".internal")
        }
    }
}

/// The only thing in the app that talks to The Modding Community.
///
/// Everything about it exists to keep one property true: **the webview never
/// holds a credential.** The frontend calls `api_get`/`api_post` with a path
/// and gets parsed JSON back; the `Authorization` header is added here, the
/// refresh dance happens here, and a 401 is resolved here rather than being
/// handed to React to figure out.
///
/// The base URL is **not a setting and never a parameter from the webview**,
/// because a "which server?" field is a phishing primitive: point the app at a
/// look-alike host and it will happily send that host a bearer token. See
/// [`api_base`] for the two ways a developer — not a user, and not a script —
/// can aim a build somewhere else.
///
/// `refresh_lock` is here because refresh-token reuse detection means two
/// concurrent refreshes would sign the user out. It makes the second caller
/// wait for the first's result.
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

        Ok(format!("{}/api/app/v1{path}", api_base()))
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
            .post(format!("{}/api/app/v1/auth/refresh", api_base()))
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

        assert!(url.starts_with(api_base()));
        assert!(url.contains("/api/app/v1/browse"));
    }

    #[test]
    fn an_override_is_reduced_to_a_bare_origin() {
        for (raw, want) in [
            ("https://tmcdev.net:3002", "https://tmcdev.net:3002"),
            ("https://tmcdev.net:3002/", "https://tmcdev.net:3002"),
            ("HTTPS://TMCDEV.NET:3002", "https://tmcdev.net:3002"),
            // The default port is dropped, so one host cannot resolve to two
            // bases that compare unequal.
            ("https://tmcdev.net:443", "https://tmcdev.net"),
            ("http://localhost:3000", "http://localhost:3000"),
            ("http://127.0.0.1:3000", "http://127.0.0.1:3000"),
            ("http://192.168.1.20:3000", "http://192.168.1.20:3000"),
        ] {
            assert_eq!(normalise_base(raw).as_deref(), Ok(want), "{raw}");
        }
    }

    #[test]
    fn an_override_that_could_send_a_token_somewhere_odd_is_refused() {
        for bad in [
            // Plaintext to anything routable: the bearer would cross the
            // network in the clear.
            "http://moddingcommunity.com",
            "http://tmcdev.net:3002",
            // Not a base.
            "ftp://tmcdev.net",
            "tmcdev.net:3002",
            "javascript:alert(1)",
            "",
            "   ",
            // Everything below concatenates `/api/app/v1…` onto the base, so a
            // path, a query or a fragment would silently reshape every URL.
            "https://tmcdev.net/staging",
            "https://tmcdev.net/?x=1",
            "https://tmcdev.net/#x",
            // Credentials in a base end up in every request line and every log.
            "https://user:pw@tmcdev.net",
        ] {
            assert!(normalise_base(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn a_scope_is_none_on_production_and_a_tag_otherwise() {
        // Whatever this build was pointed at, the two must agree — the secure
        // store keys on `api_base_scope`, and a `None` on a dev base would
        // share production's stored refresh token.
        assert_eq!(api_base_is_prod(), api_base_scope().is_none());

        if let Some(scope) = api_base_scope() {
            assert!(scope.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        }
    }
}
