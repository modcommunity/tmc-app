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
    /// A dual-stack server's native UDP (ENet) port, for `{udpPort}`. The
    /// website reads it from the server's own query; behind a TLS proxy it is
    /// not the join port, so it cannot be derived from `port`.
    pub udp_port: Option<u16>,
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
        /*
         * A separator only ever comes from the template. A substituted value
         * that IS one (`{opt:x}` = `--`, a locale of `++`) would move where
         * the engine's arguments end, so it counts as no value at all.
         */
        let value = substitute(element, ctx).filter(|v| is_separator(element) || !is_separator(v));

        match value {
            Some(value) => out.push(value),
            None => {
                /*
                 * The element before it goes too, when it looks like the flag
                 * this value belonged to. A bare `--connect` left on its own is
                 * an argument a game is entitled to reject outright, which
                 * would turn "no server chosen" into "the game will not start".
                 */
                if out
                    .last()
                    .is_some_and(|prev| prev.starts_with('-') && !is_separator(prev))
                {
                    out.pop();
                }
            }
        }
    }

    out
}

/// The arguments that tell a TMC game shell where to keep downloaded content.
///
/// Appended after the game's own template, and only when that template carries
/// the bare `--`: those are the arguments the shell reads through
/// `OS.get_cmdline_user_args()`, and a build whose template has no separator is
/// not a TMC shell and would be handed flags it never asked for.
///
/// **The folder is always the app's, never the player's choice.** "Clear game
/// downloads" deletes it recursively, and a path a user picked is a path that
/// can be their home directory. So the app names `<app data>/game-cache` on
/// every launch -- which is also how it knows exactly what to clear -- and the
/// setting is whether to keep anything there at all.
///
/// `keep == false` is `--content-cache off`: the shell downloads into a folder
/// of its own that it empties at launch and at quit. `limit_mb` of 0 leaves the
/// shell's own ceiling (4 GiB on a desktop).
pub fn cache_args(
    args: &[String],
    keep: bool,
    dir: &std::path::Path,
    limit_mb: u32,
) -> Vec<String> {
    if !args.iter().any(|a| is_separator(a)) {
        return Vec::new();
    }

    if !keep {
        return vec!["--content-cache".into(), "off".into()];
    }

    let mut out = vec![format!("--cloud-cache-dir={}", dir.to_string_lossy())];

    if limit_mb > 0 {
        out.push(format!(
            "--cloud-cache-bytes={}",
            u64::from(limit_mb) * 1024 * 1024
        ));
    }

    out
}

/// The argument that keeps a TMC game shell off UDP, when the player said so.
///
/// Empty when UDP is allowed: the shell's own `auto` already tries ENet first and
/// falls back to WebSocket, and saying nothing keeps that one decision in one place.
/// Like [`cache_args`], only a template with the bare `--` is a TMC shell.
/// `--transport ws` also outranks any `--udp` the template produced, so the pair
/// is left alone rather than removed here.
pub fn transport_args(args: &[String], use_udp: bool) -> Vec<String> {
    if use_udp || !args.iter().any(|a| is_separator(a)) {
        return Vec::new();
    }

    vec!["--transport".into(), "ws".into()]
}

/// The site the game's server browser should list, for a TMC game shell.
///
/// Native clients have no page origin, so without this a shipped client lists no
/// site servers. `site` is the app's own API base (the origin the shell's
/// `client/auth.json` names too). Only a template with the bare `--` is a TMC shell;
/// an explicit `--browser-backbone-url` already in the arguments, or a
/// `DOT_BROWSER_BACKBONE_URL` in the environment (`env_set`), wins, because the
/// game reads argv before the environment and must not overrule the player.
pub fn browser_site_args(args: &[String], site: &str, env_set: bool) -> Vec<String> {
    let site = site.trim();

    if site.is_empty()
        || env_set
        || !args.iter().any(|a| is_separator(a))
        || args
            .iter()
            .any(|a| a == "--browser-backbone-url" || a.starts_with("--browser-backbone-url="))
    {
        return Vec::new();
    }

    vec!["--browser-backbone-url".into(), site.into()]
}

/// The Solo pack a serverless launch should run: the game pack's
/// `<owner>/<repo>` id and the version to mount, `None` for the newest.
/// From the site's boot descriptor (`boot.solo`) or a `tmc://` launch link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoloPack {
    pub content_id: String,
    pub version: Option<String>,
}

impl SoloPack {
    /// A pack id and optional version, or `None` when either could not be one
    /// argument. The id is `<owner>/<repo>` in a closed character set and the
    /// version a closed one too: neither may start with `-` (it would be read as
    /// a flag), and nothing outside the set (whitespace, control characters)
    /// gets through. Both come from the site or from a `tmc://` link a web page
    /// could have written, and they are about to be process arguments.
    pub fn new(content_id: &str, version: Option<&str>) -> Option<Self> {
        let word = |s: &str, max: usize, extra: &[char]| {
            !s.is_empty()
                && s.len() <= max
                && !s.starts_with('-')
                && s.chars().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') || extra.contains(&c)
                })
        };

        let (owner, repo) = content_id.split_once('/')?;

        if !word(owner, 64, &[]) || !word(repo, 100, &[]) {
            return None;
        }

        // An empty version is "newest", like an absent one.
        let version = match version.filter(|v| !v.is_empty()) {
            Some(v) if word(v, 64, &['+']) => Some(v.to_string()),
            Some(_) => return None,
            None => None,
        };

        Some(Self {
            content_id: content_id.to_string(),
            version,
        })
    }

    /// Read `boot.solo` out of a `/play/launch` answer's `boot` object.
    ///
    /// `None` for a null or absent `solo`, and for one [`SoloPack::new`] refuses.
    pub fn from_boot(boot: &serde_json::Value) -> Option<Self> {
        let solo = boot.get("solo")?.as_object()?;

        Self::new(
            solo.get("contentId")?.as_str()?,
            solo.get("version").and_then(serde_json::Value::as_str),
        )
    }
}

/// Where a serverless launch fetches its packs: `<boot.game.origin>/content`.
///
/// A join is told where packs live by the server it joins, and the web player
/// uses its page's own `/content`, so a native shell started for solo play has
/// neither and fails at its first `latest.json` with a relative URL. The game's
/// origin from the site's own boot descriptor is the page the web player would
/// run on, and its `/content` is the same place that player fetches from. Only an
/// `https` origin with nothing after the host is taken: it is about to be a
/// process argument, and the shell still verifies every manifest against the key
/// in its export, so the worst a wrong one does is fail to find the game.
pub fn content_url_from_boot(boot: &serde_json::Value) -> Option<String> {
    let raw = boot.get("game")?.get("origin")?.as_str()?.trim();
    let url = url::Url::parse(raw).ok()?;

    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }

    Some(format!("{}/content", url.origin().ascii_serialization()))
}

/// `--content-url <base>` for a TMC game shell started for solo play. Empty for
/// a template without the bare `--`, for no base, or when the template already
/// names one (the publisher's choice wins).
pub fn content_url_args(args: &[String], base: Option<&str>) -> Vec<String> {
    let Some(base) = base else {
        return Vec::new();
    };

    if !args.iter().any(|a| is_separator(a))
        || args
            .iter()
            .any(|a| a == "--content-url" || a.starts_with("--content-url="))
    {
        return Vec::new();
    }

    vec!["--content-url".into(), base.into()]
}

/// `--solo <owner>/<repo>` (and `--solo-version <v>`) for a TMC game shell
/// started with no server. Empty for a template without the bare `--`, like
/// [`cache_args`]: only a TMC shell reads its game arguments after it.
pub fn solo_args(args: &[String], solo: Option<&SoloPack>) -> Vec<String> {
    let Some(solo) = solo else {
        return Vec::new();
    };

    if !args.iter().any(|a| is_separator(a)) {
        return Vec::new();
    }

    let mut out = vec!["--solo".to_string(), solo.content_id.clone()];

    if let Some(v) = &solo.version {
        out.push("--solo-version".into());
        out.push(v.clone());
    }

    out
}

/// [`solo_args`], or why a launch that has a solo pack cannot carry it.
///
/// The error is for the player: a template with no bare `--` would otherwise
/// start the shell at its own connect screen with nothing said.
pub fn solo_args_or_why(args: &[String], solo: &SoloPack) -> Result<Vec<String>, &'static str> {
    let out = solo_args(args, Some(solo));

    if out.is_empty() {
        return Err("This build's launch template has no `--` separator, so the game cannot be passed to it. The build needs republishing.");
    }

    Ok(out)
}

/// The first shell build date (`1.YYYYMMDD.HHMM`) whose `--solo` flag starts a pack.
pub const SOLO_SHELL_SINCE: u32 = 20261031;

/// Whether a shell build predates `--solo`, which it would ignore and open its menu.
///
/// A version that does not read as `major.date.time` is not called old: only a
/// build known to be stale is refused.
pub fn shell_predates_solo(version: &str) -> bool {
    version
        .split('.')
        .nth(1)
        .and_then(|d| d.parse::<u32>().ok())
        .is_some_and(|d| d >= 20000101 && d < SOLO_SHELL_SINCE)
}

/// The bare `--` (or Godot's `++`) that ends the engine's own arguments.
///
/// **It is never the flag a missing value belonged to.** A Godot export reads
/// its game's arguments from `OS.get_cmdline_user_args()`, which is ONLY what
/// follows this separator — and the engine silently ignores an argument it does
/// not recognise, so `tmc.x86_64 --connect host:port` starts, draws its menu and
/// connects to nothing. The published template is therefore
/// `["--", "--connect", "{host}:{port}"]`, and a template such as
/// `["--", "{host}:{port}"]` with no server must keep its separator rather than
/// have it mistaken for a flag and popped: dropping it would turn every later
/// game argument into an engine argument.
pub fn is_separator(arg: &str) -> bool {
    arg == "--" || arg == "++"
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
            "udpPort" => ctx.udp_port.map(|p| p.to_string()),
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
            udp_port: None,
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

    /// The exact template `dot-server-deploy`'s `./server export-native`
    /// prints for its native client, which reads `--connect` from
    /// `OS.get_cmdline_user_args()` — only what follows the bare `--`.
    fn godot_template() -> Vec<String> {
        vec!["--".into(), "--connect".into(), "{host}:{port}".into()]
    }

    #[test]
    fn the_published_godot_template_puts_the_address_after_the_separator() {
        assert_eq!(
            build_args(&godot_template(), &ctx()),
            vec!["--", "--connect", "play.example.com:6064"]
        );
    }

    /// The template once a build joins over UDP as well: `--udp` names the
    /// ENet port, and the shell falls back to `--connect` without it.
    fn godot_udp_template() -> Vec<String> {
        vec![
            "--".into(),
            "--connect".into(),
            "{host}:{port}".into(),
            "--udp".into(),
            "{host}:{udpPort}".into(),
        ]
    }

    #[test]
    fn a_dual_stack_server_is_joined_over_udp() {
        let mut c = ctx();
        c.udp_port = Some(6070);

        assert_eq!(
            build_args(&godot_udp_template(), &c),
            vec![
                "--",
                "--connect",
                "play.example.com:6064",
                "--udp",
                "play.example.com:6070"
            ]
        );
    }

    #[test]
    fn a_server_with_no_udp_port_drops_the_flag_and_keeps_the_join() {
        assert_eq!(
            build_args(&godot_udp_template(), &ctx()),
            vec!["--", "--connect", "play.example.com:6064"]
        );
    }

    #[test]
    fn tcp_only_puts_transport_ws_after_the_separator_and_keeps_the_join() {
        let mut c = ctx();
        c.udp_port = Some(6070);

        let mut args = build_args(&godot_udp_template(), &c);
        args.extend(transport_args(&args, false));

        let sep = args.iter().position(|a| a == "--").unwrap();
        let flag = args.iter().position(|a| a == "--transport").unwrap();

        assert!(flag > sep);
        assert_eq!(args[flag + 1], "ws");
        assert!(args.contains(&"--connect".to_string()));
    }

    #[test]
    fn the_published_godot_template_with_no_server_starts_at_the_menu() {
        let mut c = ctx();
        c.host = None;
        c.port = None;
        c.server_id = None;

        // `--connect` goes with its value; the separator stays, and a lone
        // `--` is something every Godot build accepts and ignores.
        assert_eq!(build_args(&godot_template(), &c), vec!["--"]);
    }

    #[test]
    fn a_separator_is_never_popped_as_if_it_were_a_flag() {
        let mut c = ctx();
        c.host = None;

        assert_eq!(
            build_args(
                &[
                    "--headless".into(),
                    "--".into(),
                    "{host}:{port}".into(),
                    "--name".into(),
                    "{opt:name}".into(),
                    "++".into(),
                    "{host}".into(),
                ],
                &c,
            ),
            vec!["--headless", "--", "++"]
        );
        assert!(is_separator("--") && is_separator("++"));
        assert!(!is_separator("--connect") && !is_separator("-"));
    }

    #[test]
    fn a_substituted_value_can_never_become_a_separator() {
        let mut c = ctx();
        c.options.insert("name".into(), "--".into());
        c.locale = Some("++".into());

        assert_eq!(
            build_args(
                &[
                    "--lang".into(),
                    "{locale}".into(),
                    "--".into(),
                    "--name".into(),
                    "{opt:name}".into(),
                    "--x".into(),
                    "a{opt:name}".into(),
                ],
                &c,
            ),
            vec!["--", "--x", "a--"]
        );
    }

    #[test]
    fn a_port_with_no_host_is_not_an_address() {
        let mut c = ctx();
        c.host = Some(String::new());

        assert_eq!(build_args(&godot_template(), &c), vec!["--"]);
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

    #[test]
    fn browser_site_follows_the_separator_and_yields_to_an_explicit_choice() {
        let shell = vec!["--".to_string(), "--connect".into(), "h:1".into()];
        let site = "https://moddingcommunity.com";

        assert_eq!(
            browser_site_args(&shell, site, false),
            vec!["--browser-backbone-url", site]
        );
        assert!(browser_site_args(&shell, site, true).is_empty());
        assert!(browser_site_args(&shell, "  ", false).is_empty());
        assert!(browser_site_args(&["--connect".into()], site, false).is_empty());

        let mut own = shell.clone();
        own.push("--browser-backbone-url=https://x.example".into());
        assert!(browser_site_args(&own, site, false).is_empty());
    }

    #[test]
    fn a_solo_launch_fetches_from_the_games_own_origin() {
        let boot = |origin: &str| serde_json::json!({ "game": { "origin": origin } });

        assert_eq!(
            content_url_from_boot(&boot("https://dotgames.org")),
            Some("https://dotgames.org/content".to_string())
        );
        assert_eq!(
            content_url_from_boot(&boot("https://dotgames.org/")),
            Some("https://dotgames.org/content".to_string())
        );

        // A port is part of the origin, so it is kept.
        assert_eq!(
            content_url_from_boot(&boot("https://h:8443")),
            Some("https://h:8443/content".to_string())
        );
        // A backslash is a path separator to a URL parser, so it cannot smuggle
        // a second host in: the host stays the first one and the path refuses it.
        assert_eq!(
            content_url_from_boot(&boot("https:\\\\dotgames.org")),
            Some("https://dotgames.org/content".to_string())
        );

        for bad in [
            "https://dotgames.org\\godot",
            "https://dotgames.org\\@evil.example",
            "https://evil.example\\.dotgames.org",
            "http://dotgames.org",
            "https://dotgames.org/godot",
            "https://dotgames.org?x=1",
            "https://user@dotgames.org",
            "file:///etc",
            "-x",
            "",
        ] {
            assert_eq!(content_url_from_boot(&boot(bad)), None, "{bad}");
        }

        assert_eq!(content_url_from_boot(&serde_json::json!({})), None);

        let shell: Vec<String> = vec!["--".into()];
        let base = Some("https://dotgames.org/content");
        assert_eq!(
            content_url_args(&shell, base),
            vec![
                "--content-url".to_string(),
                "https://dotgames.org/content".into()
            ]
        );
        assert!(content_url_args(&shell, None).is_empty());
        assert!(content_url_args(&["-x".to_string()], base).is_empty());
        let named: Vec<String> = vec!["--".into(), "--content-url=https://a.example/c".into()];
        assert!(content_url_args(&named, base).is_empty());
        // The value goes over as one argv element, backslash and all: there is
        // no shell to read it, so it is never split or unescaped.
        let odd = Some("https://h:8443/content\\x");
        assert_eq!(
            content_url_args(&shell, odd),
            vec![
                "--content-url".to_string(),
                "https://h:8443/content\\x".into()
            ]
        );
        let spaced: Vec<String> = vec![
            "--".into(),
            "--content-url".into(),
            "https://a.example/c".into(),
        ];
        assert!(content_url_args(&spaced, base).is_empty());
    }

    #[test]
    fn cache_args_follow_the_separator_and_only_a_shell_gets_them() {
        let dir = std::path::Path::new("/data/game-cache");
        let shell: Vec<String> = vec!["--".into(), "--connect".into(), "h:1".into()];

        assert_eq!(
            cache_args(&shell, true, dir, 0),
            vec!["--cloud-cache-dir=/data/game-cache".to_string()]
        );
        assert_eq!(
            cache_args(&shell, true, dir, 2048),
            vec![
                "--cloud-cache-dir=/data/game-cache".to_string(),
                format!("--cloud-cache-bytes={}", 2048u64 * 1024 * 1024),
            ]
        );
        assert_eq!(
            cache_args(&shell, false, dir, 2048),
            vec!["--content-cache".to_string(), "off".to_string()]
        );
        // No separator: not a TMC shell, and nothing is added.
        assert!(cache_args(&["--connect".into()], true, dir, 0).is_empty());
    }

    #[test]
    fn a_solo_pack_adds_solo_args_after_the_separator() {
        let args: Vec<String> = vec!["--".into()];
        let solo = SoloPack::from_boot(&serde_json::json!({
            "solo": { "contentId": "tmc/arena", "version": "1.2.0" }
        }));

        assert_eq!(
            solo_args(&args, solo.as_ref()),
            ["--solo", "tmc/arena", "--solo-version", "1.2.0"]
        );

        let latest = SoloPack::from_boot(&serde_json::json!({
            "solo": { "contentId": "tmc/arena", "version": null }
        }));

        assert_eq!(solo_args(&args, latest.as_ref()), ["--solo", "tmc/arena"]);
    }

    #[test]
    fn a_shell_built_before_solo_is_known_to_be_stale() {
        assert!(shell_predates_solo("1.20261010.0720"));
        assert!(!shell_predates_solo("1.20261031.0001"));
        assert!(!shell_predates_solo("1.20261104.0259"));
        assert!(!shell_predates_solo("2.0"));
        assert!(!shell_predates_solo("nightly"));
    }

    #[test]
    fn a_template_without_the_separator_is_refused_with_a_reason() {
        let solo = SoloPack::new("tmc/arena", None).unwrap();

        let err = solo_args_or_why(&["--connect".to_string()], &solo).unwrap_err();
        assert!(err.contains("--"));
        assert!(err.contains("republishing"));
        assert_eq!(
            solo_args_or_why(&["--".to_string()], &solo).unwrap(),
            ["--solo", "tmc/arena"]
        );
    }

    #[test]
    fn no_solo_or_no_separator_or_a_bad_id_adds_nothing() {
        let args: Vec<String> = vec!["--".into()];
        let ok = SoloPack::from_boot(&serde_json::json!({
            "solo": { "contentId": "tmc/arena", "version": null }
        }));

        assert!(solo_args(&args, None).is_empty());
        assert!(solo_args(&["--connect".to_string()], ok.as_ref()).is_empty());
        assert!(SoloPack::from_boot(&serde_json::json!({ "solo": null })).is_none());
        assert!(SoloPack::from_boot(&serde_json::json!({})).is_none());

        for bad in ["", "--evil", "tmc", "a/-b", "a b/c", "a/b\nc", "a/b/c"] {
            let boot = serde_json::json!({ "solo": { "contentId": bad, "version": null } });
            assert!(SoloPack::from_boot(&boot).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn udp_turned_off_tells_only_a_shell_to_use_the_websocket() {
        let shell: Vec<String> = vec!["--".into(), "--connect".into(), "h:1".into()];

        // Allowed is the shell's own choice, so nothing is said.
        assert!(transport_args(&shell, true).is_empty());
        assert_eq!(
            transport_args(&shell, false),
            vec!["--transport".to_string(), "ws".to_string()]
        );
        // No separator: not a TMC shell, and nothing is added.
        assert!(transport_args(&["--connect".into()], false).is_empty());
    }
}
