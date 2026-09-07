use serde::Serialize;

/// The single error type crossing the IPC boundary.
///
/// Every variant is something the UI can act on, and the `Display` text is
/// written to be shown to a person. Anything genuinely internal collapses into
/// [`AppError::Internal`], whose message is a fixed sentence — the detail goes
/// to the audit log and the tracing output, never to the webview. A webview has
/// no user for a stack trace, and it is the one place a hostile plugin's UI
/// could read one back.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("You are not signed in.")]
    NotAuthenticated,

    #[error("Sign-in was rejected or expired. Try again.")]
    AuthRejected,

    #[error("{0}")]
    Network(String),

    #[error("The server rejected the request: {0}")]
    Api(String),

    #[error("{0}")]
    Invalid(String),

    /// A plugin asked for a file its manifest does not permit it to touch.
    /// Always surfaced verbatim: the user needs to know which plugin and which
    /// permission, otherwise "install failed" is unactionable.
    ///
    /// Named for [`crate::plugins::jail`] rather than for a "sandbox", because
    /// in this app a **sandbox** is the user's mod profile and this error has
    /// nothing to do with one — a message reading "blocked by the sandbox"
    /// would send somebody to look at their profile settings.
    #[error("Blocked by the plugin file jail: {0}")]
    Jail(String),

    /// Carries its detail privately — `Display` is the generic sentence, and
    /// [`AppError::detail`] is what the audit log writes.
    #[error("Something went wrong. Check the activity log for details.")]
    Internal(String),
}

impl AppError {
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::Invalid(msg.into())
    }

    pub fn jail(msg: impl Into<String>) -> Self {
        Self::Jail(msg.into())
    }

    /// The stable discriminant the frontend switches on. The message may be
    /// reworded at any time; this may not.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAuthenticated => "not_authenticated",
            Self::AuthRejected => "auth_rejected",
            Self::Network(_) => "network",
            Self::Api(_) => "api",
            Self::Invalid(_) => "invalid",
            Self::Jail(_) => "jail",
            Self::Internal(_) => "internal",
        }
    }

    /// The full detail, for the audit log only.
    pub fn detail(&self) -> String {
        match self {
            Self::Internal(src) => src.clone(),
            other => other.to_string(),
        }
    }
}

#[derive(Serialize)]
struct WireError {
    code: &'static str,
    message: String,
}

/// Serialised as `{ code, message }` so TypeScript narrows on `code` rather
/// than matching prose.
impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        WireError {
            code: self.code(),
            message: self.to_string(),
        }
        .serialize(s)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::internal(format!("io: {e}"))
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::internal(format!("json: {e}"))
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        // Timeouts and connection failures are ordinary offline conditions and
        // read as such; anything else is a bug on one side or the other.
        if e.is_timeout() || e.is_connect() {
            Self::Network("Could not reach The Modding Community. Check your connection.".into())
        } else {
            Self::internal(format!("http: {e}"))
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;
