//! The avatar studio's privileged half (docs/avatar.md).
//!
//! The studio (`@modcommunity/shared/avatar`) renders in the webview, but the
//! webview never fetches: it names one of three fixed `/api/avatar/v1` routes,
//! and for a model file it names a URL that Rust then checks against an
//! allowlist it rebuilt itself from TMC's own answers. That URL is the one
//! place this app takes a URL from the webview, and the check is here, not in
//! the caller: a URL TMC did not name in the manifest, the game's library or
//! the member's own wearables is refused, so `avatar_asset` is never an open
//! proxy and never a way to reach a LAN address.
//!
//! The allowlist mirrors `AssetAllowlist` in the shared package (and
//! website-gs's `/api/avatar-asset`), so the two clients agree on what a
//! model file is.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;
use url::Url;

/// The `/api/avatar/v1` routes the app speaks. The webview picks a command,
/// never a path; each command names its route here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarRoute {
    /// The built-in figure: model file and per-slot options. Public.
    Manifest,
    /// One game's avatar library (`?app=`). Public.
    Library,
    /// The member's avatar for one game (`GET ?app=`, `PUT`).
    Me,
}

impl AvatarRoute {
    pub fn path(self) -> &'static str {
        match self {
            Self::Manifest => "manifest",
            Self::Library => "library",
            Self::Me => "me",
        }
    }
}

/// `raw` made absolute against `base` (the API's origin), or None when it is
/// not a web URL this app may fetch: `https` always, `http` only when the base
/// itself is `http` (a local dev site).
pub fn absolute(raw: &str, base: &str) -> Option<String> {
    let raw = raw.trim();

    if raw.is_empty() {
        return None;
    }

    let base = Url::parse(&format!("{}/", base.trim_end_matches('/'))).ok()?;
    let url = base.join(raw).ok()?;

    let ok = match url.scheme() {
        "https" => true,
        "http" => base.scheme() == "http",
        _ => false,
    };

    if !ok || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }

    Some(url.to_string())
}

/// Every model file TMC named: the manifest's figure and slot options, the
/// library's avatars and wearables, and (when given) the member's own
/// wearables from `GET /api/app/v1/avatar/wearables`.
pub fn allowlist(
    manifest: Option<&Value>,
    library: Option<&Value>,
    own_wearables: Option<&Value>,
    base: &str,
) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut add = |v: Option<&Value>| {
        if let Some(u) = v.and_then(Value::as_str).and_then(|u| absolute(u, base)) {
            out.insert(u);
        }
    };

    if let Some(m) = manifest {
        add(m.get("modelUrl"));

        for slot in array(m.get("slots")) {
            for option in array(slot.get("options")) {
                add(option.get("model"));
            }
        }
    }

    for doc in [library, own_wearables].into_iter().flatten() {
        for key in ["avatars", "wearables"] {
            for item in array(doc.get(key)) {
                add(item.get("url"));
            }
        }
    }

    out
}

/// Whether the webview's `url` is one TMC named. Compared after the same
/// normalisation the list went through, so `https://Host/a` and
/// `https://host/a` are one file.
pub fn is_allowed(list: &HashSet<String>, url: &str, base: &str) -> bool {
    absolute(url, base).is_some_and(|u| list.contains(&u))
}

fn array(v: Option<&Value>) -> impl Iterator<Item = &Value> {
    v.and_then(Value::as_array).into_iter().flatten()
}

/// How long a public allowlist (manifest plus one library) is reused before
/// TMC is asked again: the stage loads several files at once, and each would
/// otherwise cost two API calls. The member's own wearables are never cached.
pub const ALLOW_TTL: Duration = Duration::from_secs(60);

/// Public allowlists by app id (0 = the manifest alone).
#[derive(Default)]
pub struct AllowCache {
    inner: Mutex<HashMap<u64, (Instant, HashSet<String>)>>,
}

impl AllowCache {
    pub fn get(&self, app: u64) -> Option<HashSet<String>> {
        let map = self.inner.lock().ok()?;
        let (at, set) = map.get(&app)?;

        (at.elapsed() < ALLOW_TTL).then(|| set.clone())
    }

    pub fn put(&self, app: u64, set: HashSet<String>) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(app, (Instant::now(), set));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BASE: &str = "https://tmcdev.net";

    #[test]
    fn relative_files_resolve_against_the_api_and_odd_schemes_are_refused() {
        assert_eq!(
            absolute("/file/a.glb", BASE).as_deref(),
            Some("https://tmcdev.net/file/a.glb")
        );
        assert_eq!(absolute("http://cdn.example/a.glb", BASE), None);
        assert_eq!(absolute("file:///etc/passwd", BASE), None);
        assert_eq!(absolute("blob:https://x/1", BASE), None);
        assert_eq!(absolute("https://u:p@cdn.example/a.glb", BASE), None);
        assert_eq!(absolute("   ", BASE), None);
        assert_eq!(
            absolute("http://localhost:3002/a.glb", "http://localhost:3002").as_deref(),
            Some("http://localhost:3002/a.glb")
        );
    }

    #[test]
    fn the_list_is_what_tmc_named_and_nothing_else() {
        let manifest = json!({
            "modelUrl": "https://cdn.example/figure.glb",
            "slots": [{ "options": [{ "model": "/file/hat.glb" }, { "model": null }] }],
        });
        let library = json!({
            "avatars": [{ "url": "https://cdn.example/robot.vrm" }],
            "wearables": [{ "url": "/file/cape.glb" }, { "name": "no url" }],
        });
        let own = json!({ "wearables": [{ "url": "/file/mine.glb" }] });

        let public = allowlist(Some(&manifest), Some(&library), None, BASE);

        for u in [
            "https://cdn.example/figure.glb",
            "https://tmcdev.net/file/hat.glb",
            "https://cdn.example/robot.vrm",
            "https://tmcdev.net/file/cape.glb",
        ] {
            assert!(is_allowed(&public, u, BASE), "{u}");
        }

        assert!(!is_allowed(
            &public,
            "https://tmcdev.net/file/mine.glb",
            BASE
        ));
        assert!(!is_allowed(&public, "http://192.168.1.1/admin", BASE));
        assert!(!is_allowed(
            &public,
            "https://cdn.example/figure.glb/../x",
            BASE
        ));

        let with_own = allowlist(Some(&manifest), None, Some(&own), BASE);

        assert!(is_allowed(
            &with_own,
            "https://tmcdev.net/file/mine.glb",
            BASE
        ));
        assert!(!is_allowed(
            &with_own,
            "https://cdn.example/robot.vrm",
            BASE
        ));
    }

    #[test]
    fn junk_documents_give_an_empty_list() {
        let junk = json!({ "modelUrl": 3, "slots": "no", "avatars": {} });

        assert!(allowlist(Some(&junk), Some(&junk), Some(&junk), BASE).is_empty());
    }

    #[test]
    fn the_cache_keeps_lists_per_app() {
        let cache = AllowCache::default();

        cache.put(7, HashSet::from(["a".to_string()]));

        assert!(cache.get(7).is_some_and(|s| s.contains("a")));
        assert!(cache.get(8).is_none());
    }
}
