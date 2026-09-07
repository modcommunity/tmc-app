//! **`tmc.json`** — what a mod archive says about where it came from.
//!
//! An archive downloaded from the website and an archive downloaded from a
//! forum post are the same bytes to this app: a zip with some files in it. The
//! difference matters — one of them can be linked back to an item page, kept up
//! to date and reported against, and the other cannot — and nothing in a zip's
//! own format carries it. So the website puts a small JSON document at the
//! archive's root and this module reads it.
//!
//! WHAT IT IS, AND WHAT IT IS EMPHATICALLY NOT
//! -------------------------------------------
//! It is a **hint about identity**. It names the item, the release and the game
//! so a dropped file becomes "Cool Mod 1.4" with a link rather than
//! "cool-mod-1.4.zip".
//!
//! It is **not a permission, not a signature and not an instruction**. It
//! arrives inside a file the user dragged in from somewhere, so every field is
//! attacker-controlled text:
//!
//!   * `installPath` is run through [`join_relative`] like any other path from
//!     a plugin — a metadata file naming `../../..` places nothing;
//!   * `source` is compared against [`api_base`] to decide whether the item
//!     link is OFFERED, and a mismatch downgrades the import to an ordinary
//!     local mod rather than failing it. An archive claiming to be from the
//!     site when it is not gets no more trust than one claiming nothing;
//!   * nothing here selects an install rule, widens a jail or reaches the
//!     network.
//!
//! The honest summary is that `tmc.json` saves the user some typing. That is
//! worth having and it is all it is.
//!
//! WHY NOT SIGN IT
//! ---------------
//! A signature would answer "did the site really produce this archive?", and
//! the app already has the machinery ([`crate::plugins::signature`]). It would
//! also be the wrong shape: an item's release file is authored by the mod's
//! author and served by the site, the site does not re-pack it, and a signing
//! key that has to touch every uploaded file is a key on a web server. The
//! thing worth verifying about a downloaded release is its **checksum**, which
//! the API already carries and [`crate::download`] already enforces on the path
//! where the app fetched the file itself.
//!
//! [`join_relative`]: crate::plugins::jail::join_relative
//! [`api_base`]: crate::api::api_base

use serde::{Deserialize, Serialize};

/// The file at an archive's root.
pub const METADATA_FILE: &str = "tmc.json";

/// The alternative location, for archives that would rather keep their root
/// clean. Checked second; a bundle carrying both wins with the root copy.
pub const METADATA_FILE_ALT: &str = ".tmc/metadata.json";

/// The only version this build reads.
pub const METADATA_VERSION: u32 = 1;

/// Cap on the sidecar's size. It holds a dozen short strings; anything larger
/// is either a mistake or an attempt to make the parser the expensive part of
/// opening an archive.
pub const MAX_METADATA_BYTES: u64 = 64 * 1024;

/// What an archive claims about itself.
///
/// Every field past `metadataVersion` is optional, and that is not laziness:
/// this document is written by several producers over time — the website today,
/// an author packaging by hand tomorrow — and a required field is a field that
/// makes every older archive unreadable the day it is added. A sidecar naming
/// only `name` is perfectly useful.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModMetadata {
    pub metadata_version: u32,

    /// The site that produced this archive, as an origin (`https://host`).
    /// Compared against this build's API base before any item link is offered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// `mod`, `asset` — the content kind on the site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_id: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_url: Option<String>,

    /// The game, by URL slug and/or numeric id. The slug is what selects
    /// `plugins/app/<slug>/…`; the id is what everything stores.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<i64>,

    /// Where the archive's contents belong relative to the game folder —
    /// `mods`, `BepInEx/plugins`, or empty for the game's root.
    ///
    /// A hint, and validated like any other untrusted path before use. When it
    /// is absent the importer falls back to the game's own `sandbox.json`
    /// `modTargets`, which is the answer the app would have used anyway.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_path: Option<String>,

    /// `client`, `server` or `shared`, matching a sandbox's environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
}

impl ModMetadata {
    /// Parse the sidecar's bytes.
    ///
    /// Returns `None` for anything this build cannot read — a future version, a
    /// truncated file, JSON that is not an object. A malformed sidecar must
    /// never fail the import: the archive's FILES are what the user dragged in,
    /// and refusing them because a description file is broken would be the app
    /// caring more about its own bookkeeping than about the user's mod.
    pub fn parse(raw: &[u8]) -> Option<Self> {
        if raw.len() as u64 > MAX_METADATA_BYTES {
            return None;
        }

        let parsed: Self = serde_json::from_slice(raw).ok()?;

        if parsed.metadata_version != METADATA_VERSION {
            return None;
        }

        Some(parsed.trimmed())
    }

    /// Bound every string, because all of them end up in a database column, a
    /// window title and an audit line.
    fn trimmed(mut self) -> Self {
        fn cap(field: &mut Option<String>, max: usize) {
            if let Some(value) = field {
                let cleaned: String = value
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(max)
                    .collect();

                *field = if cleaned.trim().is_empty() {
                    None
                } else {
                    Some(cleaned.trim().to_string())
                };
            }
        }

        cap(&mut self.source, 256);
        cap(&mut self.kind, 32);
        cap(&mut self.name, 200);
        cap(&mut self.version, 64);
        cap(&mut self.author, 120);
        cap(&mut self.description, 2000);
        cap(&mut self.web_url, 512);
        cap(&mut self.app_slug, 120);
        cap(&mut self.install_path, 512);
        cap(&mut self.environment, 16);

        self
    }

    /// Does this sidecar claim to come from the site this build talks to?
    ///
    /// Compared by ORIGIN, not by string equality: a sidecar written as
    /// `https://moddingcommunity.com/` and an API base of
    /// `https://moddingcommunity.com` name the same site, and a trailing slash
    /// is not a reason to drop an item link.
    ///
    /// A sidecar with no `source` at all is treated as NOT ours. Absence is not
    /// assent — an archive somebody packed by hand with a copied `tmc.json`
    /// should not inherit a link to whatever item id it happens to name.
    pub fn is_from(&self, api_base: &str) -> bool {
        let Some(claimed) = &self.source else {
            return false;
        };

        match (url::Url::parse(claimed), url::Url::parse(api_base)) {
            (Ok(a), Ok(b)) => a.origin() == b.origin(),
            _ => false,
        }
    }

    /// The item this describes, when it describes one from THIS site.
    ///
    /// `(kind, itemId)` together or not at all — half a reference is a link the
    /// UI cannot build and a subscription the app cannot check.
    pub fn item_ref(&self, api_base: &str) -> Option<(String, i64)> {
        if !self.is_from(api_base) {
            return None;
        }

        match (&self.kind, self.item_id) {
            (Some(kind), Some(id)) if !kind.is_empty() && id > 0 => Some((kind.clone(), id)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> serde_json::Value {
        serde_json::json!({
            "metadataVersion": 1,
            "source": "https://moddingcommunity.com/",
            "kind": "mod",
            "itemId": 1234,
            "releaseId": 88,
            "name": "Cool Mod",
            "version": "1.4.0",
            "appSlug": "minecraft",
            "installPath": "mods"
        })
    }

    #[test]
    fn a_well_formed_sidecar_parses() {
        let raw = serde_json::to_vec(&sample()).expect("json");
        let parsed = ModMetadata::parse(&raw).expect("parses");

        assert_eq!(parsed.name.as_deref(), Some("Cool Mod"));
        assert_eq!(parsed.item_id, Some(1234));
        assert_eq!(parsed.install_path.as_deref(), Some("mods"));
    }

    #[test]
    fn a_future_version_is_ignored_rather_than_guessed_at() {
        let mut doc = sample();
        doc["metadataVersion"] = serde_json::json!(2);

        let raw = serde_json::to_vec(&doc).expect("json");

        assert!(ModMetadata::parse(&raw).is_none());
    }

    #[test]
    fn garbage_is_ignored_rather_than_fatal() {
        assert!(ModMetadata::parse(b"not json at all").is_none());
        assert!(ModMetadata::parse(b"[1, 2, 3]").is_none());
        assert!(ModMetadata::parse(&[]).is_none());
    }

    #[test]
    fn an_oversized_sidecar_is_refused_without_parsing() {
        let big = vec![b'{'; (MAX_METADATA_BYTES + 1) as usize];

        assert!(ModMetadata::parse(&big).is_none());
    }

    /// The whole point of `source`: an item link is offered only for an archive
    /// that claims the site this build actually talks to.
    #[test]
    fn an_item_link_needs_a_matching_origin() {
        let raw = serde_json::to_vec(&sample()).expect("json");
        let parsed = ModMetadata::parse(&raw).expect("parses");

        assert_eq!(
            parsed.item_ref("https://moddingcommunity.com"),
            Some(("mod".to_string(), 1234))
        );

        // A dev build must not inherit a production archive's item link — the
        // ids are not the same database.
        assert_eq!(parsed.item_ref("https://tmcdev.net"), None);
    }

    #[test]
    fn a_sidecar_with_no_source_claims_nothing() {
        let mut doc = sample();
        doc["source"] = serde_json::Value::Null;

        let raw = serde_json::to_vec(&doc).expect("json");
        let parsed = ModMetadata::parse(&raw).expect("parses");

        assert!(!parsed.is_from("https://moddingcommunity.com"));
        assert_eq!(parsed.item_ref("https://moddingcommunity.com"), None);
    }

    #[test]
    fn control_characters_do_not_reach_a_name() {
        let mut doc = sample();
        doc["name"] = serde_json::json!("Cool\u{0}\nMod");

        let raw = serde_json::to_vec(&doc).expect("json");
        let parsed = ModMetadata::parse(&raw).expect("parses");

        assert_eq!(parsed.name.as_deref(), Some("CoolMod"));
    }
}
