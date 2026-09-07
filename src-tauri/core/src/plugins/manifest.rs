//! What a plugin is allowed to say about itself.
//!
//! The central design decision, and the one everything else follows from:
//! **a plugin is data, never code.** There is no script engine, no WASM host,
//! no `exec` step. An installer describes a sequence of file operations; a
//! query plugin describes a datagram and how to read the reply; a theme
//! describes colour tokens. Everything a plugin can express is something this
//! crate implements and audits.
//!
//! That is a real limitation — a plugin cannot do anything we did not think of
//! — and it is the point. The alternative is running third-party code with the
//! user's full filesystem access on the same machine as their games, their save
//! files and their credentials, and no amount of isolation bolted on
//! afterwards recovers from that starting position.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

/// The only manifest version this build accepts.
pub const MANIFEST_VERSION: u32 = 1;

/// File a plugin bundle must contain at its root.
pub const MANIFEST_FILE: &str = "plugin.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub manifest_version: u32,

    /// Reverse-DNS id. Also the on-disk directory name, which is why
    /// [`is_valid_plugin_id`] is strict about it.
    pub id: String,

    pub name: String,
    pub version: String,
    pub author: String,

    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,

    /// TMC app (game) ids this plugin handles. Empty means "any", which is
    /// only sensible for a theme.
    #[serde(default)]
    pub apps: Vec<i64>,

    pub permissions: Permissions,

    #[serde(default)]
    pub installer: Option<Installer>,
    #[serde(default)]
    pub server_query: Option<ServerQuery>,
    #[serde(default)]
    pub theme: Option<Theme>,

    /// A mod manager this app can read an existing library out of.
    ///
    /// The fourth plugin type, and the one that reads rather than writes:
    /// [`crate::plugins::managers`] describes where another manager keeps its
    /// mods, and the app copies from there into its own store. It declares no
    /// steps and needs no `fs` grant, because a scan never writes and the
    /// copy is performed by the app against paths the SCAN found — see that
    /// module's header for why a descriptor cannot name a path of its own.
    #[serde(default)]
    pub manager: Option<crate::plugins::managers::ManagerSpec>,
}

/// Everything a plugin may reach. Absent means denied — there is no wildcard
/// and no "inherit" that could widen a grant.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Permissions {
    /// Writable roots. Each entry names a base the app resolves (never a path
    /// the plugin supplies) plus an optional subdirectory under it.
    #[serde(default)]
    pub fs: Vec<FsGrant>,

    /// Hosts the plugin may download from. Exact hostnames or a single leading
    /// `*.` label — never a bare `*`, and never a scheme other than https.
    #[serde(default)]
    pub net: Vec<String>,

    /// UDP/TCP query targets are NOT listed here: a query plugin probes the
    /// server the user picked, which is not knowable at authoring time. The
    /// control there is [`crate::net::resolve_public`], which refuses anything
    /// that is not a public address.
    #[serde(default)]
    pub query: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FsRoot {
    /// The configured install directory for the app being handled. Resolved
    /// from `AppSettings::game_dirs`, so the USER chose it.
    GameDir,
    /// This plugin's own private directory under the app's data dir.
    PluginData,
    /// The user's configured download directory. Read-write, but shared with
    /// other plugins — anything durable belongs in `pluginData`.
    Downloads,
}

/// One subtree of one root the plugin may touch.
///
/// Several grants may name the same root — `{gameDir, "mods", write}` and
/// `{gameDir, "config", write}` — and they compose as a union of permitted
/// subtrees rather than merging. A `PathRef` is always resolved against the
/// ROOT and then checked against these, so declaring narrow access stays narrow
/// however many grants there are and in whatever order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FsGrant {
    pub root: FsRoot,
    /// Subdirectory under the root, e.g. `mods`. Empty means the root itself.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub write: bool,
}

// ------------------------------------------------------------------ Installer

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installer {
    /// Which content kinds this installer handles: `mod`, `asset`, or both.
    #[serde(default)]
    pub kinds: Vec<String>,
    pub install: Vec<Step>,
    #[serde(default)]
    pub uninstall: Vec<Step>,
}

/// One action in an install or uninstall plan.
///
/// Note what is NOT here: no shell, no process spawn, no environment access, no
/// arbitrary path. `Download` is the only step that touches the network and it
/// is bounded by the manifest's host allow-list; every path in every other step
/// is a *relative* path resolved against a granted root.
/*
 * `rename_all_fields` as well as `rename_all`: the former renames each
 * variant's FIELDS, the latter only the variant tags. Without it a manifest
 * would have to write `max_bytes`, which no other key in the format does.
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Step {
    /// Fetch a file into the jail.
    ///
    /// `url` may contain `{}`-placeholders filled from the install context
    /// (the release file's URL, the item id). It is re-checked against the
    /// allow-list AFTER substitution, so a placeholder cannot smuggle a host in.
    Download {
        url: String,
        to: PathRef,
        /// Expected SHA-256, when the plugin author knows it. A mismatch aborts
        /// the whole plan.
        #[serde(default)]
        sha256: Option<String>,
        #[serde(default)]
        max_bytes: Option<u64>,
    },

    /// Unpack a `.zip`, `.tar`, `.tar.gz` inside the jail.
    Extract {
        from: PathRef,
        to: PathRef,
        /// Drop this many leading path components from each entry.
        #[serde(default)]
        strip: u8,
        /// Only extract entries whose path starts with one of these prefixes.
        #[serde(default)]
        include: Vec<String>,
    },

    Copy {
        from: PathRef,
        to: PathRef,
    },

    Move {
        from: PathRef,
        to: PathRef,
    },

    Mkdir {
        path: PathRef,
    },

    /// Delete a file, or a directory the plan itself created.
    Remove {
        path: PathRef,
    },

    /// Write a UTF-8 file. Content may contain context placeholders.
    WriteText {
        path: PathRef,
        content: String,
    },

    /// Set one value in a JSON file, creating it if absent.
    ///
    /// A whole-file write is what a plugin would otherwise have to do to change
    /// one setting, and that silently discards everything else in the user's
    /// config. `pointer` is an RFC 6901 JSON Pointer.
    PatchJson {
        path: PathRef,
        pointer: String,
        value: serde_json::Value,
    },
}

/// A path, always relative and always anchored to a granted root.
///
/// There is no variant for an absolute path. That is not an oversight — the
/// type simply cannot express one, so no amount of manifest creativity produces
/// a write outside a root the user granted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathRef {
    pub root: FsRoot,
    /// Relative to the ROOT — not to any one `FsGrant` — and `/`-separated.
    /// So a plugin granted `{gameDir, "mods"}` writes to
    /// `{ root: gameDir, path: "mods/foo.jar" }`. Validated by
    /// [`crate::plugins::jail::Jail::resolve`].
    pub path: String,
}

// -------------------------------------------------------------- Server query

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerQuery {
    /// `udp` or `tcp`. Only `udp` is implemented; `tcp` is reserved.
    pub protocol: String,
    /// Which port to use: `game`, `query`, or a fixed number.
    #[serde(default)]
    pub port: PortSource,
    /// Hex request payload. May contain `{challenge}` for two-stage protocols.
    pub request_hex: String,
    #[serde(default = "default_query_timeout")]
    pub timeout_ms: u64,
    /// How to read the reply.
    pub parse: Vec<Field>,
}

fn default_query_timeout() -> u64 {
    1_500
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PortSource {
    #[default]
    Query,
    Game,
    Fixed(u16),
}

/// One value to pull out of a query response.
///
/// A tiny read-only grammar rather than a regex or an expression language: a
/// parser a plugin can drive is a parser a plugin can hang the app with, and
/// every variant here consumes a bounded number of bytes from a bounded buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum Field {
    /// Skip forward.
    Skip {
        bytes: u32,
    },
    U8 {
        name: String,
    },
    U16Le {
        name: String,
    },
    U16Be {
        name: String,
    },
    U32Le {
        name: String,
    },
    U32Be {
        name: String,
    },
    /// NUL-terminated string.
    CString {
        name: String,
    },
    /// Length-prefixed string, prefix is a single byte.
    PString {
        name: String,
    },
    /// Assert the next bytes equal this hex value, else the parse fails.
    Magic {
        hex: String,
    },
}

// --------------------------------------------------------------------- Theme

/// A theme is a token map, not a stylesheet.
///
/// Arbitrary CSS would let a plugin load a remote font or background image and
/// turn every app launch into a beacon to the author's server — and would let
/// it reposition or hide UI, including the buttons that uninstall it. A fixed
/// list of custom properties with strictly-validated values can do neither.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Theme {
    pub label: String,
    /// `light` or `dark` — which base the tokens are overriding.
    #[serde(default)]
    pub base: Option<String>,
    /// `--token` → value. Both sides are validated; see
    /// [`crate::plugins::theme`].
    pub tokens: BTreeMap<String, String>,
}

// ---------------------------------------------------------------- Validation

/// Reverse-DNS-ish, and safe as a single path component on every platform.
///
/// Strict because this string becomes a directory name: no separators, no
/// dots-only names, no Windows reserved device names, no leading/trailing dot
/// or space (which Windows silently strips, producing a different directory
/// than the one that was checked).
pub fn is_valid_plugin_id(id: &str) -> bool {
    if id.len() < 3 || id.len() > 96 {
        return false;
    }

    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' || c == '_')
    {
        return false;
    }

    if id.starts_with('.') || id.ends_with('.') || id.contains("..") {
        return false;
    }

    const RESERVED: [&str; 6] = ["con", "prn", "aux", "nul", "com1", "lpt1"];

    !RESERVED.contains(&id)
}

/// A host entry: `example.com` or `*.example.com`. Nothing else.
fn is_valid_host_pattern(pattern: &str) -> bool {
    let host = pattern.strip_prefix("*.").unwrap_or(pattern);

    // A bare `*` or an empty label after the wildcard would match everything.
    if host.is_empty() || host == "*" || host.contains('*') {
        return false;
    }

    if host.len() > 253 || !host.contains('.') {
        return false;
    }

    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

impl Manifest {
    /// Parse and validate. A manifest that fails here is never stored, so an
    /// installed plugin is one that has already passed every check below.
    pub fn parse(raw: &str) -> AppResult<Self> {
        if raw.len() > 256 * 1024 {
            return Err(AppError::invalid("Plugin manifest is too large."));
        }

        // `deny_unknown_fields` throughout: a manifest carrying a key we do not
        // understand is either from a newer app version or is trying something,
        // and both deserve a refusal rather than a silent ignore.
        let manifest: Manifest = serde_json::from_str(raw)
            .map_err(|e| AppError::invalid(format!("Plugin manifest is invalid: {e}")))?;

        manifest.validate()?;

        Ok(manifest)
    }

    fn validate(&self) -> AppResult<()> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(AppError::invalid(format!(
                "Plugin manifest version {} is not supported by this app version.",
                self.manifest_version
            )));
        }

        if !is_valid_plugin_id(&self.id) {
            return Err(AppError::invalid(
                "Plugin id must be lowercase letters, digits, '.', '-' or '_'.",
            ));
        }

        for field in [&self.name, &self.version, &self.author] {
            if field.trim().is_empty() || field.len() > 128 {
                return Err(AppError::invalid(
                    "Plugin name, version and author are required.",
                ));
            }
        }

        if self.installer.is_none() && self.server_query.is_none() && self.theme.is_none() {
            return Err(AppError::invalid("Plugin declares no capability."));
        }

        for host in &self.permissions.net {
            if !is_valid_host_pattern(host) {
                return Err(AppError::invalid(format!(
                    "'{host}' is not a valid host. Use 'example.com' or '*.example.com'."
                )));
            }
        }

        if let Some(installer) = &self.installer {
            if installer.install.len() > 128 || installer.uninstall.len() > 128 {
                return Err(AppError::invalid("Plugin has too many steps."));
            }

            /*
             * A plan with no write grant can only fail, loudly, halfway
             * through. Catching it at install time means the user is told
             * "this plugin is broken" rather than "installing Foo failed".
             */
            if !installer.install.is_empty() && !self.permissions.fs.iter().any(|g| g.write) {
                return Err(AppError::invalid(
                    "Plugin has install steps but no writable filesystem permission.",
                ));
            }
        }

        if let Some(q) = &self.server_query {
            if q.protocol != "udp" {
                return Err(AppError::invalid(
                    "Only 'udp' server queries are supported.",
                ));
            }

            if !self.permissions.query {
                return Err(AppError::invalid(
                    "Plugin declares a server query but not the 'query' permission.",
                ));
            }

            if q.parse.len() > 64 {
                return Err(AppError::invalid("Query parser has too many fields."));
            }
        }

        if let Some(theme) = &self.theme {
            crate::plugins::theme::validate(theme)?;
        }

        Ok(())
    }

    /// Whether `host` is covered by the net allow-list.
    pub fn allows_host(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();

        self.permissions.net.iter().any(|pattern| {
            let pattern = pattern.to_ascii_lowercase();

            match pattern.strip_prefix("*.") {
                // `*.example.com` covers `a.example.com` but NOT `example.com`
                // itself, and not `notexample.com` — the dot is part of the
                // suffix on purpose.
                Some(suffix) => host.ends_with(&format!(".{suffix}")),
                None => host == pattern,
            }
        })
    }

    /// The identity the user approves.
    ///
    /// Over the manifest's canonical bytes, not over the bundle: what the user
    /// is consenting to is the declared permissions and steps. Re-serialising
    /// through serde first means whitespace and key order cannot change the
    /// hash, so a re-download of the same plugin does not re-prompt.
    pub fn fingerprint(&self) -> String {
        let canonical = serde_json::to_vec(self).unwrap_or_default();

        hex::encode(Sha256::digest(canonical))
    }

    /// A plain-language list of what approving this plugin allows, for the
    /// consent dialog. The dialog shows exactly this — no summarising, no
    /// "and 3 more".
    pub fn permission_summary(&self) -> Vec<String> {
        let mut out = Vec::new();

        for grant in &self.permissions.fs {
            let where_ = match grant.root {
                FsRoot::GameDir => "the game's install folder",
                FsRoot::PluginData => "its own private folder",
                FsRoot::Downloads => "your downloads folder",
            };

            let sub = if grant.path.is_empty() {
                String::new()
            } else {
                format!(" ({})", grant.path)
            };

            out.push(format!(
                "{} files in {where_}{sub}",
                if grant.write {
                    "Read and write"
                } else {
                    "Read"
                }
            ));
        }

        for host in &self.permissions.net {
            out.push(format!("Download files from {host}"));
        }

        if self.permissions.query {
            out.push("Send queries to game servers you view".into());
        }

        if self.theme.is_some() {
            out.push("Change the app's colours".into());
        }

        /*
         * Named as a READ, because that is all it is and the difference is the
         * whole reason somebody would agree to it. A manager descriptor lists
         * directories under a base the app resolved; it writes nothing, and
         * importing from what it found is a separate click on a separate
         * screen.
         */
        if let Some(manager) = &self.manager {
            out.push(format!(
                "Look for installed mods in {}'s own folders",
                manager.label
            ));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ids_reject_path_tricks() {
        for bad in [
            "..",
            "a/b",
            "a\\b",
            ".hidden",
            "trailing.",
            "a..b",
            "con",
            "UPPER",
            "x",
        ] {
            assert!(!is_valid_plugin_id(bad), "{bad} should be rejected");
        }

        assert!(is_valid_plugin_id("com.example.installer"));
        assert!(is_valid_plugin_id("my-plugin_2"));
    }

    #[test]
    fn host_patterns_cannot_widen_to_everything() {
        for bad in ["*", "*.", "*.com*", "", "localhost", "a..b.com"] {
            assert!(!is_valid_host_pattern(bad), "{bad} should be rejected");
        }

        assert!(is_valid_host_pattern("example.com"));
        assert!(is_valid_host_pattern("*.example.com"));
    }

    /// The examples are the documentation. A manifest field renamed without
    /// updating them would ship a reference that does not load.
    #[test]
    fn the_shipped_examples_all_validate() {
        // `core/` → `src-tauri/` → repo root.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");

        let entries = std::fs::read_dir(&root)
            .unwrap_or_else(|e| panic!("no examples at {}: {e}", root.display()));

        let mut checked = 0;

        for entry in entries.flatten() {
            let path = entry.path().join(MANIFEST_FILE);

            if !path.exists() {
                continue;
            }

            let raw = std::fs::read_to_string(&path).expect("readable");

            Manifest::parse(&raw).unwrap_or_else(|e| panic!("{} is invalid: {e}", path.display()));

            checked += 1;
        }

        assert!(
            checked >= 3,
            "expected the example plugins, found {checked}"
        );
    }

    #[test]
    fn wildcard_matches_subdomains_only() {
        let m = Manifest {
            manifest_version: 1,
            id: "a.b".into(),
            name: "t".into(),
            version: "1".into(),
            author: "t".into(),
            description: None,
            homepage: None,
            apps: vec![],
            permissions: Permissions {
                net: vec!["*.example.com".into()],
                ..Default::default()
            },
            installer: None,
            server_query: None,
            theme: None,
            manager: None,
        };

        assert!(m.allows_host("cdn.example.com"));
        assert!(!m.allows_host("example.com"));
        assert!(!m.allows_host("notexample.com"));
        assert!(!m.allows_host("example.com.evil.net"));
    }
}
