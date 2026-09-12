//! What the site answers when this device asks for a build, and how its launch
//! arguments become an argv vector.
//!
//! THE DEVICE NAMES AN ID AND ITS OWN PLATFORM, AND NOTHING ELSE
//! ------------------------------------------------------------
//! `GET /apps/:id/build?platform=…` is the install half of `/play/launch`, and
//! it follows the same rule `download_release` does: the webview names ids, and
//! the URL is read here. There is no `game_install(url)` anywhere, so there is
//! nothing an injected script could point at an archive of its choosing.
//!
//! WHAT IS RE-CHECKED ON THIS SIDE, AND WHY
//! ---------------------------------------
//! The site checks the scheme and the digest shape before it publishes a row.
//! Both are checked again here, because a check that lives only on the other
//! side of a network hop is a check this process is trusting somebody else to
//! have run — and what is at the end of this one is an archive that gets
//! unpacked and executed.
//!
//! THE ARGUMENTS ARE A VECTOR, NEVER A COMMAND LINE
//! -----------------------------------------------
//! `args` arrives as one element per argument and stays that way. A command
//! string would have to be split by somebody, and there is no shell in this
//! app to do the splitting — so a server name containing a space would become
//! two arguments, and a server name containing a quote would become something
//! nobody can predict. Substitution happens INSIDE an element and can never
//! create a new one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

use super::platform::BuildPlatform;

/// How the artifact is packed. Mirrors `AppBuildFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BuildFormat {
    Zip,
    TarGz,
    /// The artifact IS the executable — a single static binary, an `.apk`.
    /// Unpacking one would destroy the thing being installed.
    Raw,
}

impl BuildFormat {
    /// The extension the downloaded file is given.
    ///
    /// It matters: `plugins::steps::extract_archive` dispatches on the file's
    /// NAME, so a `.tar.gz` saved as `download.bin` is refused as "not an
    /// archive". Naming the temporary file from the declared format is what
    /// makes the one extraction implementation — with its zip-slip guard, its
    /// expansion cap and tar's link refusal — reachable from here without a
    /// second copy of any of it.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::TarGz => "tar.gz",
            Self::Raw => "bin",
        }
    }

    pub fn is_archive(self) -> bool {
        !matches!(self, Self::Raw)
    }
}

/// One resolved build, exactly as `/apps/:id/build` answers it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeBuild {
    pub app_id: i64,
    pub platform: BuildPlatform,
    pub version: String,
    pub url: String,
    /// Lower-case hex SHA-256. Never optional — see [`NativeBuild::check`].
    pub sha256: String,
    pub size_bytes: u64,
    pub format: BuildFormat,
    /// Relative path of the executable inside the archive; null for `RAW`.
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub min_client_version: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Cap on one game archive.
///
/// The download queue has its own 32 GB ceiling and this is far under it, on
/// purpose: that one is a backstop against a server lying about a file, and
/// this is a statement about what a game published here plausibly is. A build
/// past it is far more likely to be a mistake in a publishing script than a
/// game somebody meant to ship.
pub const MAX_BUILD_BYTES: u64 = 24 * 1024 * 1024 * 1024;

impl NativeBuild {
    /// Everything that has to be true before a byte is fetched.
    ///
    /// One function rather than checks scattered down the install, so the
    /// refusals can be read as a list — and so that a new one cannot be added
    /// to a path that some other caller skips.
    pub fn check(&self, client_version: &str) -> AppResult<()> {
        if !self.url.starts_with("https://") {
            return Err(AppError::invalid(
                "That build is not served over HTTPS and will not be downloaded.",
            ));
        }

        /*
         * The digest is checked for SHAPE here and for VALUE by the download
         * queue, after the last byte and before the file is given its real
         * name. Checking the shape first is what turns "a publishing script
         * wrote the word `pending` into the column" into a refusal before the
         * download rather than after it.
         */
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::invalid(
                "That build does not carry a usable checksum, so it will not be installed.",
            ));
        }

        if self.sha256.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(AppError::invalid(
                "That build's checksum is not in the expected form.",
            ));
        }

        if self.size_bytes == 0 || self.size_bytes > MAX_BUILD_BYTES {
            return Err(AppError::invalid(
                "That build's size is not one the app will download.",
            ));
        }

        if self.format.is_archive() && self.entry.as_deref().unwrap_or_default().trim().is_empty() {
            return Err(AppError::invalid(
                "That build does not say which file to start, so it cannot be installed.",
            ));
        }

        if let Some(minimum) = self.min_client_version.as_deref() {
            if !minimum.trim().is_empty() && !crate::version::meets_minimum(client_version, minimum)
            {
                return Err(AppError::invalid(format!(
                    "That build needs version {minimum} of the app or newer. Update the app first."
                )));
            }
        }

        Ok(())
    }
}

/// What a launch knows about where it is going.
///
/// Every field optional, because every one of them is: a game started from its
/// own menu has no server, and a game with no options has no options.
#[derive(Debug, Clone, Default)]
pub struct LaunchContext {
    pub app_id: i64,
    /// The app's URL slug, for `{app}`.
    pub app_slug: Option<String>,
    pub server_id: Option<i64>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub locale: Option<String>,
    pub options: BTreeMap<String, String>,
}

/// Characters a substituted value may never contain.
///
/// A NUL truncates an argument for every C API underneath us, and a carriage
/// return or newline is what turns one argument into something a log, a
/// terminal or a game's own config parser reads as two. None of them appears in
/// a hostname, a port or a legitimate option value, so refusing is free.
fn clean(value: &str) -> Option<String> {
    if value
        .bytes()
        .any(|b| b == 0 || b == b'\n' || b == b'\r' || b < 0x20)
    {
        return None;
    }

    Some(value.to_string())
}

/// Substitute the tokens in a launch template.
///
/// The same vocabulary the website's `PLAY_URI_TOKENS` names, so an operator
/// writing `args` for a build and writing `playAppUri` for the same game is
/// writing the same tokens.
///
/// **An argument whose value is missing is DROPPED, along with the argument
/// before it when that one is a flag.** `["--connect", "{host}:{port}"]` with no
/// server must not become `["--connect", ":"]` — the game would take that as an
/// address, fail to reach it, and report a connection error for a connection
/// nobody asked for. Dropping the pair starts the game at its own menu, which
/// is what "launch with no server" means.
pub fn build_args(template: &[String], ctx: &LaunchContext) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(template.len());

    for element in template {
        match substitute(element, ctx) {
            Some(value) => out.push(value),
            None => {
                /*
                 * The element before it goes too, when it looks like the flag
                 * this value belonged to. A bare `--connect` left on its own is
                 * an argument a game is entitled to reject outright, which
                 * would turn "no server chosen" into "the game will not start".
                 */
                if out.last().is_some_and(|prev| prev.starts_with('-')) {
                    out.pop();
                }
            }
        }
    }

    out
}

/// One element, or `None` when a token in it had no value.
fn substitute(element: &str, ctx: &LaunchContext) -> Option<String> {
    let mut out = String::with_capacity(element.len());
    let mut rest = element;

    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);

        let Some(end) = rest[start..].find('}').map(|i| start + i) else {
            // An unclosed brace is not a token. Left verbatim, exactly as the
            // website leaves an unrecognised one — a typo should be visible in
            // the result rather than silently eaten.
            out.push_str(&rest[start..]);

            return Some(out);
        };

        let token = &rest[start + 1..end];

        let value = match token {
            "appId" => Some(ctx.app_id.to_string()),
            "app" => ctx
                .app_slug
                .clone()
                .filter(|s| !s.is_empty())
                .or_else(|| Some(ctx.app_id.to_string())),
            "serverId" => ctx.server_id.map(|id| id.to_string()),
            "host" => ctx.host.clone().filter(|h| !h.is_empty()),
            "port" => ctx.port.map(|p| p.to_string()),
            "locale" => ctx.locale.clone().filter(|l| !l.is_empty()),
            other => match other.strip_prefix("opt:") {
                Some(key) => ctx.options.get(key).cloned(),
                // An unrecognised token is left verbatim, braces and all.
                None => Some(format!("{{{other}}}")),
            },
        }?;

        out.push_str(&clean(&value)?);

        rest = &rest[end + 1..];
    }

    out.push_str(rest);

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> LaunchContext {
        LaunchContext {
            app_id: 42,
            app_slug: Some("hungario".into()),
            server_id: Some(7),
            host: Some("play.example.com".into()),
            port: Some(6064),
            locale: Some("en".into()),
            options: BTreeMap::new(),
        }
    }

    fn build() -> NativeBuild {
        NativeBuild {
            app_id: 42,
            platform: BuildPlatform::LinuxX64,
            version: "1.0.0".into(),
            url: "https://cdn.example.com/a.zip".into(),
            sha256: "a".repeat(64),
            size_bytes: 1024,
            format: BuildFormat::Zip,
            entry: Some("game.x86_64".into()),
            args: vec![],
            min_client_version: None,
            notes: None,
        }
    }

    #[test]
    fn an_address_becomes_one_argument() {
        let args = build_args(&["--connect".into(), "{host}:{port}".into()], &ctx());

        assert_eq!(args, vec!["--connect", "play.example.com:6064"]);
    }

    #[test]
    fn no_server_drops_the_flag_rather_than_connecting_to_nothing() {
        let mut c = ctx();
        c.host = None;
        c.port = None;
        c.server_id = None;

        let args = build_args(
            &[
                "--game".into(),
                "{app}".into(),
                "--connect".into(),
                "{host}:{port}".into(),
            ],
            &c,
        );

        // The game still knows which game it is; it simply starts at its menu.
        assert_eq!(args, vec!["--game", "hungario"]);
    }

    #[test]
    fn a_value_can_never_become_a_second_argument() {
        let mut c = ctx();
        c.options.insert("name".into(), "two words --admin".into());

        let args = build_args(&["--name".into(), "{opt:name}".into()], &c);

        assert_eq!(args, vec!["--name", "two words --admin"]);
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn a_control_character_drops_the_argument() {
        let mut c = ctx();
        c.host = Some("evil\nhost".into());

        assert_eq!(
            build_args(&["--connect".into(), "{host}".into()], &c),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_unknown_token_survives_verbatim() {
        assert_eq!(
            build_args(&["{nosuchtoken}".into()], &ctx()),
            vec!["{nosuchtoken}"]
        );
        assert_eq!(build_args(&["{unclosed".into()], &ctx()), vec!["{unclosed"]);
    }

    #[test]
    fn a_build_without_a_usable_checksum_is_refused() {
        let mut b = build();
        b.sha256 = "pending".into();

        assert!(b.check("1.0.0").is_err());

        b.sha256 = "A".repeat(64);
        assert!(b.check("1.0.0").is_err());
    }

    #[test]
    fn a_plaintext_build_is_refused() {
        let mut b = build();
        b.url = "http://cdn.example.com/a.zip".into();

        assert!(b.check("1.0.0").is_err());
    }

    #[test]
    fn an_archive_with_nothing_to_start_is_refused() {
        let mut b = build();
        b.entry = None;

        assert!(b.check("1.0.0").is_err());

        // A single-file build is the one case where that is correct.
        b.format = BuildFormat::Raw;
        assert!(b.check("1.0.0").is_ok());
    }

    #[test]
    fn a_build_needing_a_newer_app_is_refused_before_it_is_fetched() {
        let mut b = build();
        b.min_client_version = Some("2.0.0".into());

        assert!(b.check("1.5.0").is_err());
        assert!(b.check("2.0.0").is_ok());
    }
}
