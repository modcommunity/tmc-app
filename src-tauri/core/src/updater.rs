//! Where the app's OWN updates come from, and which key they must carry.
//!
//! The Tauri plugin does the downloading and the minisign check; this module
//! only decides the two inputs, from three places in a fixed order:
//!
//! | Input | Override (Settings → App → Updates) | Compiled in | Last resort |
//! | --- | --- | --- | --- |
//! | endpoint | `updaterEndpoint` | `TMC_UPDATER_ENDPOINT` | the site's `/api/app/v1/update/…` |
//! | public key | `updaterPubkey` | `TMC_UPDATER_PUBKEY` | **none — no updater** |
//!
//! The compiled endpoint is the static `latest.json` the release workflow
//! uploads to the S3 bucket (`downloads/gamespice-app/latest.json`), which works
//! with nothing on the site's side. The site's endpoint remains the fallback
//! for a build made without one, and the override lets an operator point a
//! machine at either — or at a staging bucket — without a rebuild.
//!
//! WHAT THE OVERRIDES COST, STATED PLAINLY
//! ---------------------------------------
//! The signature is the updater's entire security model, and a public-key
//! override is a way to change which signatures are trusted. Anything that can
//! write this app's settings can therefore choose what the app installs next.
//! That was asked for — a fork, a staging channel or a rotated key must not
//! need a rebuild to test — and it is bounded the way the jail anchors are:
//!
//!   * `settings_patch` REFUSES both fields ([`crate::settings::UPDATER_SOURCE_FIELDS`]);
//!     they move only through their own command, which validates them here.
//!   * The endpoint must be `https` (plain `http` only for a loopback/LAN host
//!     in a debug build) and carry no credentials.
//!   * The key must decode as a minisign public key, so a typo is refused at
//!     the form rather than discovered as "every update fails its signature".
//!   * Every change is audited at **Security** level, so turning logging off
//!     cannot hide it, and Settings says an override is active.
//!
//! A build with NO key anywhere still has no updater — overriding is how a key
//! is added at runtime, never how a check is skipped.

use base64::Engine as _;
use serde::Serialize;

use crate::error::{AppError, AppResult};

/// The release channels. `stable` is the default and the only one most people
/// should see; `beta` reads `latest-beta.json`, which the release workflow
/// writes for pre-release tags (and for stable ones, so beta never trails).
pub const CHANNELS: &[&str] = &["stable", "beta"];

/// Longest endpoint accepted. A URL is not a document.
const MAX_ENDPOINT: usize = 2048;

/// Longest key accepted. A real one is ~120 characters of base64.
const MAX_PUBKEY: usize = 1024;

/// Where one of the two inputs came from, for the settings screen to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    /// Settings → App → Updates.
    Override,
    /// Baked into this binary at build time.
    Compiled,
    /// The site's own update route, because nothing else named one.
    Site,
}

/// The resolved inputs for one update check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterSource {
    /// With the channel applied and the plugin's `{{…}}` placeholders intact.
    pub endpoint: String,
    pub endpoint_origin: Origin,
    /// `None` means this build cannot install updates, and says so.
    pub pubkey: Option<String>,
    pub key_origin: Option<Origin>,
    pub channel: String,
}

/// What this binary was built with. `option_env!` values, passed in so the
/// resolution is testable without rebuilding.
#[derive(Debug, Clone, Copy, Default)]
pub struct Compiled<'a> {
    pub pubkey: Option<&'a str>,
    pub endpoint: Option<&'a str>,
}

/// The site's own route, for a build that names nothing better.
pub fn site_endpoint(api_base: &str) -> String {
    format!(
        "{}/api/app/v1/update/{{{{target}}}}/{{{{arch}}}}/{{{{current_version}}}}",
        api_base.trim_end_matches('/')
    )
}

/// Resolve both inputs.
///
/// An override or compiled value that fails validation is skipped rather than
/// used — a bad compiled endpoint falls back to the site, a bad compiled key to
/// no updater — so the answer is always something [`validate_endpoint`] and
/// [`validate_pubkey`] would accept.
pub fn resolve(
    endpoint_override: Option<&str>,
    pubkey_override: Option<&str>,
    channel: &str,
    compiled: Compiled<'_>,
    api_base: &str,
) -> UpdaterSource {
    let channel = if CHANNELS.contains(&channel) {
        channel
    } else {
        "stable"
    };

    let (endpoint, endpoint_origin) = endpoint_override
        .and_then(|e| validate_endpoint(e).ok())
        .map(|e| (e, Origin::Override))
        .or_else(|| {
            compiled
                .endpoint
                .and_then(|e| validate_endpoint(e).ok())
                .map(|e| (e, Origin::Compiled))
        })
        .unwrap_or_else(|| (site_endpoint(api_base), Origin::Site));

    let (pubkey, key_origin) = match pubkey_override
        .and_then(|k| validate_pubkey(k).ok())
        .map(|k| (k, Origin::Override))
        .or_else(|| {
            compiled
                .pubkey
                .and_then(|k| validate_pubkey(k).ok())
                .map(|k| (k, Origin::Compiled))
        }) {
        Some((key, origin)) => (Some(key), Some(origin)),
        None => (None, None),
    };

    UpdaterSource {
        endpoint: apply_channel(&endpoint, channel),
        endpoint_origin,
        pubkey,
        key_origin,
        channel: channel.to_string(),
    }
}

/// Point an endpoint at a channel.
///
/// `{{channel}}` anywhere is replaced; otherwise, for the beta channel, a path
/// ending in `latest.json` becomes `latest-beta.json`. Anything else — the
/// site's route — is left as it is, because the site has no channels to ask.
pub fn apply_channel(endpoint: &str, channel: &str) -> String {
    if endpoint.contains("{{channel}}") {
        return endpoint.replace("{{channel}}", channel);
    }

    if channel == "stable" {
        return endpoint.to_string();
    }

    let (path, query) = match endpoint.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (endpoint, None),
    };

    match path.strip_suffix("latest.json") {
        Some(stem) => {
            let path = format!("{stem}latest-{channel}.json");

            match query {
                Some(q) => format!("{path}?{q}"),
                None => path,
            }
        }
        None => endpoint.to_string(),
    }
}

/// Is this a URL an update may be fetched from?
///
/// Returned trimmed. The plugin's `{{target}}`, `{{arch}}`,
/// `{{current_version}}` and `{{bundle_type}}` placeholders, and this module's
/// `{{channel}}`, are allowed anywhere in the path.
pub fn validate_endpoint(raw: &str) -> AppResult<String> {
    let text = raw.trim();

    if text.is_empty() || text.len() > MAX_ENDPOINT {
        return Err(AppError::invalid(
            "That update address is empty or too long.",
        ));
    }

    if text.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(AppError::invalid(
            "An update address cannot contain spaces or control characters.",
        ));
    }

    let url = url::Url::parse(text)
        .map_err(|_| AppError::invalid("That is not a URL the updater can use."))?;

    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::invalid(
            "An update address cannot carry a username or password.",
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| AppError::invalid("That update address has no host."))?;

    match url.scheme() {
        "https" => {}
        // A local test server, in a build nobody ships. The signature still
        // decides what installs; this only decides who may be asked.
        "http" if cfg!(debug_assertions) && is_local_host(host) => {}
        _ => return Err(AppError::invalid("Updates are only fetched over HTTPS.")),
    }

    Ok(text.to_string())
}

fn is_local_host(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');

    if bare.eq_ignore_ascii_case("localhost") {
        return true;
    }

    match bare.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// Is this a public key the updater can verify with?
///
/// Tauri's key is the base64 of a minisign public-key FILE: an
/// `untrusted comment:` line, then the base64 of 42 bytes — the `Ed` algorithm
/// tag, an eight-byte key id and the 32-byte Ed25519 key. Checked to that depth
/// so a truncated paste is refused here rather than as a signature failure
/// after a download.
pub fn validate_pubkey(raw: &str) -> AppResult<String> {
    let text: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let refuse = || {
        AppError::invalid(
            "That is not an updater public key (the text `tauri signer generate` prints).",
        )
    };

    if text.is_empty() || text.len() > MAX_PUBKEY {
        return Err(refuse());
    }

    let engine = base64::engine::general_purpose::STANDARD;
    let file = engine.decode(&text).map_err(|_| refuse())?;
    let file = String::from_utf8(file).map_err(|_| refuse())?;

    let key_line = file
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .ok_or_else(refuse)?;

    let key = engine.decode(key_line).map_err(|_| refuse())?;

    if key.len() != 42 || &key[..2] != b"Ed" {
        return Err(refuse());
    }

    Ok(text)
}

/// This project's updater public key (the `TMC_UPDATER_PUBKEY` the release
/// workflow compiles in), as a known-good fixture for the tests that need a
/// real key. Public by definition; the private half is never in this tree.
#[cfg(test)]
pub(crate) const TEST_PUBKEY: &str = include_str!("../../updater.pub").trim_ascii();

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = TEST_PUBKEY;

    const S3: &str = "https://cdn.example.com/downloads/gamespice-app/latest.json";

    #[test]
    fn a_real_key_is_accepted_and_whitespace_is_ignored() {
        assert_eq!(validate_pubkey(KEY).expect("key"), KEY);

        let wrapped = format!("  {}\n{}  ", &KEY[..40], &KEY[40..]);
        assert_eq!(validate_pubkey(&wrapped).expect("wrapped"), KEY);
    }

    #[test]
    fn a_truncated_or_foreign_key_is_refused() {
        assert!(validate_pubkey(&KEY[..KEY.len() - 12]).is_err());
        assert!(validate_pubkey("").is_err());
        assert!(validate_pubkey("not base64 at all!").is_err());

        // Valid base64 of something that is not a minisign key.
        let other = base64::engine::general_purpose::STANDARD.encode("untrusted comment: x\nAAAA");
        assert!(validate_pubkey(&other).is_err());
    }

    #[test]
    fn endpoints_must_be_https_and_carry_no_credentials() {
        assert!(validate_endpoint(S3).is_ok());
        assert!(validate_endpoint(
            "https://x.example/api/app/v1/update/{{target}}/{{arch}}/{{current_version}}"
        )
        .is_ok());

        for bad in [
            "http://cdn.example.com/latest.json",
            "ftp://cdn.example.com/latest.json",
            "https://user:pw@cdn.example.com/latest.json",
            "https://cdn.example.com/a b.json",
            "latest.json",
            "",
        ] {
            assert!(validate_endpoint(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn a_debug_build_may_ask_a_local_test_server_over_http() {
        let local = validate_endpoint("http://127.0.0.1:8080/latest.json");
        assert_eq!(local.is_ok(), cfg!(debug_assertions));
        assert!(validate_endpoint("http://example.com/latest.json").is_err());
    }

    #[test]
    fn the_beta_channel_reads_its_own_manifest() {
        assert_eq!(apply_channel(S3, "stable"), S3);
        assert_eq!(
            apply_channel(S3, "beta"),
            "https://cdn.example.com/downloads/gamespice-app/latest-beta.json"
        );
        assert_eq!(
            apply_channel(&format!("{S3}?v=1"), "beta"),
            "https://cdn.example.com/downloads/gamespice-app/latest-beta.json?v=1"
        );
        assert_eq!(
            apply_channel("https://x.example/{{channel}}/latest.json", "beta"),
            "https://x.example/beta/latest.json"
        );
        // The site's route has no channels, and is left alone.
        let site = site_endpoint("https://x.example");
        assert_eq!(apply_channel(&site, "beta"), site);
    }

    #[test]
    fn the_override_wins_then_the_compiled_value_then_the_site() {
        let compiled = Compiled {
            pubkey: Some(KEY),
            endpoint: Some(S3),
        };

        let plain = resolve(None, None, "stable", compiled, "https://site.example");
        assert_eq!(plain.endpoint, S3);
        assert_eq!(plain.endpoint_origin, Origin::Compiled);
        assert_eq!(plain.key_origin, Some(Origin::Compiled));

        let over = resolve(
            Some("https://staging.example/latest.json"),
            Some(KEY),
            "beta",
            compiled,
            "https://site.example",
        );
        assert_eq!(over.endpoint, "https://staging.example/latest-beta.json");
        assert_eq!(over.endpoint_origin, Origin::Override);
        assert_eq!(over.key_origin, Some(Origin::Override));

        let bare = resolve(
            None,
            None,
            "stable",
            Compiled::default(),
            "https://site.example/",
        );
        assert_eq!(bare.endpoint, site_endpoint("https://site.example"));
        assert_eq!(bare.endpoint_origin, Origin::Site);
        assert_eq!(bare.pubkey, None, "no key anywhere is no updater");
    }

    #[test]
    fn an_invalid_value_is_skipped_rather_than_used() {
        let compiled = Compiled {
            pubkey: Some("garbage"),
            endpoint: Some("http://insecure.example/latest.json"),
        };

        let s = resolve(
            Some("also garbage"),
            Some("nope"),
            "nightly",
            compiled,
            "https://site.example",
        );

        assert_eq!(s.endpoint_origin, Origin::Site);
        assert_eq!(s.pubkey, None);
        assert_eq!(s.channel, "stable", "an unknown channel is stable");
    }
}
