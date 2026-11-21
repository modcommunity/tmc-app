use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::AppResult;

/// The activity log the Settings → Logging panel reads.
///
/// This is an AUDIT trail, not diagnostics. `tracing` handles the latter and
/// goes to stderr where only a developer sees it. What lands here is the record
/// of things the app did *on the user's behalf or on a plugin's instruction* —
/// every file written during an install, every host a plugin reached, every
/// permission granted. Two consequences of that framing:
///
///   * **It is written even when logging is "off".** The app-settings toggle
///     controls the low-severity half (`Debug`/`Info` chatter). Plugin actions
///     and security decisions are always recorded, because a switch that lets a
///     plugin ask the user to stop watching is not a security control.
///   * **It is append-only and never rewritten in place.** Rotation copies to a
///     `.1` file and truncates; there is no code path that edits an existing
///     line, so a tampered log is a truncated one, which is visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    /// A security-relevant decision: a permission granted, a plugin approved, a
    /// jail refusal. Never suppressed.
    Security,
    Warn,
    Error,
}

/// Broad buckets so the panel can filter without free-text search.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogScope {
    Auth,
    Api,
    Plugin,
    Install,
    Network,
    Settings,
    App,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// RFC 3339, UTC.
    pub at: String,
    pub level: LogLevel,
    pub scope: LogScope,
    /// Short machine-ish label, e.g. `plugin.step.download`.
    pub event: String,
    pub message: String,
    /// Which plugin, when the entry is about one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    /// Structured extras. Must never contain a token or a credential — see
    /// [`Audit::write`].
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub data: serde_json::Map<String, serde_json::Value>,
}

/// Rotate at 4 MiB. Big enough to hold a long install session, small enough
/// that the panel can read the whole thing without a streaming parser.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Cap on how many entries the panel will ever be handed at once.
const MAX_READ: usize = 5_000;

/// Mask credentials in free text before it is written or mirrored anywhere.
///
/// Handles `Bearer <token>`, `Basic <b64>`, and `key=value` / `key: value` /
/// `"key":"value"` pairs where the key names a secret (token, password, secret,
/// authorization, api key, cookie, code). The key stays so the log still says a
/// value was present.
pub fn redact(text: &str) -> String {
    const KEYS: [&str; 10] = [
        "password",
        "passwd",
        "token",
        "secret",
        "authorization",
        "api_key",
        "apikey",
        "api-key",
        "cookie",
        "rcon",
    ];

    let mut out = String::with_capacity(text.len());
    let lower = text.to_ascii_lowercase();
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < text.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }

        let rest = &lower[i..];

        // Scheme-prefixed credentials.
        let scheme = ["bearer ", "basic "].iter().find(|s| rest.starts_with(**s));

        if let Some(s) = scheme {
            let start = i + s.len();
            let end = text[start..]
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',')
                .map_or(text.len(), |n| start + n);

            if end > start {
                out.push_str(&text[i..start]);
                out.push_str("[redacted]");
                i = end;
                continue;
            }
        }

        // A secret-named key followed by a separator.
        if let Some(key) = KEYS.iter().find(|k| rest.starts_with(**k)) {
            let mut j = i + key.len();

            // Allow a suffix of key characters (`access_token`, `passwordHash`).
            while j < text.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }

            let mut k = j;

            if k < text.len() && (bytes[k] == b'"' || bytes[k] == b'\'') {
                k += 1;
            }

            while k < text.len() && bytes[k] == b' ' {
                k += 1;
            }

            if k < text.len() && matches!(bytes[k], b'=' | b':') {
                k += 1;

                while k < text.len() && (bytes[k] == b' ' || bytes[k] == b'"' || bytes[k] == b'\'')
                {
                    k += 1;
                }

                let end = text[k..]
                    .find(|c: char| {
                        c.is_whitespace() || c == '"' || c == '\'' || c == '&' || c == ','
                    })
                    .map_or(text.len(), |n| k + n);

                let schemed = lower[k..].starts_with("bearer ") || lower[k..].starts_with("basic ");

                if end > k && !schemed {
                    out.push_str(&text[i..k]);
                    out.push_str("[redacted]");
                    i = end;
                    continue;
                }
            }
        }

        let ch = text[i..].chars().next().unwrap();

        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

pub struct Audit {
    path: PathBuf,
    /// Serialises writers. The lock is held only for the duration of one
    /// `write_all`, so a long install does not block the UI's reads.
    file: Mutex<()>,
    /// Whether the low-severity half is being recorded. Security and above
    /// ignore this — see the type docs.
    verbose: Mutex<bool>,
}

impl Audit {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            file: Mutex::new(()),
            verbose: Mutex::new(true),
        }
    }

    pub fn set_verbose(&self, on: bool) {
        if let Ok(mut v) = self.verbose.lock() {
            *v = on;
        }
    }

    fn verbose(&self) -> bool {
        self.verbose.lock().map(|v| *v).unwrap_or(true)
    }

    pub fn write(&self, mut entry: LogEntry) {
        entry.message = redact(&entry.message);

        for (key, value) in entry.data.iter_mut() {
            if redact(&format!("{key}=x")) != format!("{key}=x") {
                *value = serde_json::Value::String("[redacted]".into());
            } else if let serde_json::Value::String(text) = value {
                *text = redact(text);
            }
        }

        if !self.verbose() && entry.level < LogLevel::Security {
            return;
        }

        // Mirror to tracing so a dev run shows the same story in the terminal.
        match entry.level {
            LogLevel::Error => tracing::error!(event = %entry.event, "{}", entry.message),
            LogLevel::Warn => tracing::warn!(event = %entry.event, "{}", entry.message),
            _ => tracing::info!(event = %entry.event, "{}", entry.message),
        }

        let Ok(_guard) = self.file.lock() else { return };

        if let Err(e) = self.rotate_if_needed() {
            tracing::warn!("audit rotate failed: {e}");
        }

        let Ok(mut line) = serde_json::to_string(&entry) else {
            return;
        };
        line.push('\n');

        let write = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| f.write_all(line.as_bytes()));

        if let Err(e) = write {
            // A failed audit write must not fail the operation being audited —
            // that would let a full disk turn every install into an error. It
            // does get shouted about in the diagnostics stream.
            tracing::error!("audit write failed: {e}");
        }
    }

    fn rotate_if_needed(&self) -> std::io::Result<()> {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return Ok(());
        };

        if meta.len() < MAX_BYTES {
            return Ok(());
        }

        let backup = self.path.with_extension("jsonl.1");
        std::fs::rename(&self.path, backup)?;
        File::create(&self.path)?;

        Ok(())
    }

    /// The most recent entries, newest first, optionally filtered.
    pub fn read(&self, limit: usize, level: Option<LogLevel>) -> AppResult<Vec<LogEntry>> {
        let Ok(file) = File::open(&self.path) else {
            return Ok(vec![]);
        };

        let mut out: Vec<LogEntry> = BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            // A truncated final line (crash mid-write) is skipped rather than
            // failing the whole read.
            .filter_map(|l| serde_json::from_str::<LogEntry>(&l).ok())
            .filter(|e| level.is_none_or(|min| e.level >= min))
            .collect();

        out.reverse();
        out.truncate(limit.min(MAX_READ));

        Ok(out)
    }

    pub fn clear(&self) -> AppResult<()> {
        let _guard = self.file.lock();

        if self.path.exists() {
            File::create(&self.path)?;
        }

        Ok(())
    }
}

/// `audit!(state, Info, Plugin, "plugin.enable", "Enabled {id}", plugin = id)`
///
/// A macro rather than a builder so the call site stays one line: these are
/// sprinkled through the step executor, and anything longer would not get
/// written at all.
#[macro_export]
macro_rules! audit {
    ($audit:expr, $level:ident, $scope:ident, $event:expr, $msg:expr) => {
        $audit.write($crate::logging::LogEntry {
            at: $crate::logging::now_rfc3339(),
            level: $crate::logging::LogLevel::$level,
            scope: $crate::logging::LogScope::$scope,
            event: $event.into(),
            message: $msg.into(),
            plugin: None,
            data: Default::default(),
        })
    };
    ($audit:expr, $level:ident, $scope:ident, $event:expr, $msg:expr, data = $data:expr) => {
        $audit.write($crate::logging::LogEntry {
            at: $crate::logging::now_rfc3339(),
            level: $crate::logging::LogLevel::$level,
            scope: $crate::logging::LogScope::$scope,
            event: $event.into(),
            message: $msg.into(),
            plugin: None,
            data: $data,
        })
    };
    ($audit:expr, $level:ident, $scope:ident, $event:expr, $msg:expr, plugin = $plugin:expr) => {
        $audit.write($crate::logging::LogEntry {
            at: $crate::logging::now_rfc3339(),
            level: $crate::logging::LogLevel::$level,
            scope: $crate::logging::LogScope::$scope,
            event: $event.into(),
            message: $msg.into(),
            plugin: Some($plugin.to_string()),
            data: Default::default(),
        })
    };
    ($audit:expr, $level:ident, $scope:ident, $event:expr, $msg:expr, plugin = $plugin:expr, data = $data:expr) => {
        $audit.write($crate::logging::LogEntry {
            at: $crate::logging::now_rfc3339(),
            level: $crate::logging::LogLevel::$level,
            scope: $crate::logging::LogScope::$scope,
            event: $event.into(),
            message: $msg.into(),
            plugin: Some($plugin.to_string()),
            data: $data,
        })
    };
}

/// Milliseconds since the epoch.
///
/// For throttling, never for a timestamp anybody reads — `now_rfc3339` is that.
/// A clock that jumps backwards (an NTP correction, a laptop waking with a
/// stale RTC) makes an interval check pass early, which is a wasted request
/// rather than a wrong one.
pub fn epoch_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

#[cfg(test)]
mod redact_tests {
    use super::redact;

    #[test]
    fn masks_bearer_and_basic() {
        assert_eq!(
            redact("Authorization: Bearer abc.def"),
            "Authorization: Bearer [redacted]"
        );
        assert_eq!(
            redact("sent Basic dXNlcjpwdw== ok"),
            "sent Basic [redacted] ok"
        );
    }

    #[test]
    fn masks_key_value_pairs_and_json() {
        assert_eq!(
            redact("GET /x?token=abc123&a=1"),
            "GET /x?token=[redacted]&a=1"
        );
        assert_eq!(
            redact("password: hunter2 done"),
            "password: [redacted] done"
        );
        assert_eq!(
            redact(r#"{"access_token":"zzz","name":"bob"}"#),
            r#"{"access_token":"[redacted]","name":"bob"}"#
        );
        assert_eq!(redact("rcon_password=pw"), "rcon_password=[redacted]");
    }

    #[test]
    fn leaves_ordinary_text_and_unicode_alone() {
        let s = "Installed mod \u{00e9}t\u{00e9} at /home/a/b; token count 3";

        assert_eq!(redact(s), s);
        assert_eq!(redact(""), "");
    }

    #[test]
    fn audit_write_redacts_message_and_data() {
        use super::*;

        let dir = std::env::temp_dir().join(format!("gs-log-{}", std::process::id()));

        std::fs::create_dir_all(&dir).unwrap();

        let audit = Audit::new(dir.join("a.jsonl"));
        let mut data = serde_json::Map::new();

        data.insert("password".into(), "pw".into());
        audit!(audit, Info, App, "t", "login token=abc", data = data);

        let raw = std::fs::read_to_string(dir.join("a.jsonl")).unwrap();

        assert!(!raw.contains("abc") && !raw.contains("\"pw\""), "{raw}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
