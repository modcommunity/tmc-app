//! What a `tmc://` link is allowed to mean.
//!
//! THE RULE THIS MODULE EXISTS TO KEEP
//! ----------------------------------
//! **A deep link can ask the app to SHOW something. It can never ask the app to
//! DO something.**
//!
//! Any program on the machine can claim a custom scheme, and any web page can
//! navigate to one without a click. So a link is treated as an untrusted
//! request from an unknown party, and the most it can achieve is putting a page
//! on screen with a button on it — which the user then presses, or does not.
//!
//! That is why [`DeepLink::Install`] carries a content id and nothing else. It
//! is not "install this", it is "open this item's page". A link that could
//! start an install would be a one-click remote install primitive reachable
//! from any web page in any browser, which is precisely the thing the plugin
//! model spends four hundred lines avoiding elsewhere.
//!
//! WHY AUTH STILL READS NOTHING
//! ---------------------------
//! `tmc://auth` carries no payload at all and is only a WAKE-UP: it makes the
//! app poll the API immediately instead of waiting out its interval. Nothing is
//! read out of it, because anything a link carried would be something an
//! attacker could also have written.
//!
//! THE GUEST CASE
//! --------------
//! `tmc://install/mod/1234` is what the website hands somebody who is not
//! signed in. A signed-in user gets a subscription instead, which is better —
//! it follows them to their other devices and keeps the mod updated — but a
//! guest has no account to hang one on, so the link carries the item directly.
//! Both end at the same place: the item's page, with the install button on it.

use serde::Serialize;

/// The scheme, and the only one this app registers.
pub const SCHEME: &str = "tmc";

/// Cap on a link. Nothing legitimate is close to this.
const MAX_URL: usize = 2048;

/// Cap on a numeric id, so an absurd one is refused before it becomes a request.
const MAX_ID: i64 = 1_000_000_000;

/// What a link resolved to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum DeepLink {
    /// `tmc://auth` — poll now. Carries nothing, deliberately.
    Auth,

    /// `tmc://install/<kind>/<id>` — open an item's page.
    ///
    /// Named `Install` because that is what the link is FOR, and misnaming it
    /// would be the first step towards somebody making it live up to the name.
    /// It opens a page.
    Install { kind: String, id: i64 },

    /// `tmc://view/<kind>/<id>` — the same thing, without the intent.
    View { kind: String, id: i64 },

    /// `tmc://sandbox/<id>` — open one sandbox.
    ///
    /// Local ids only, so this is only ever useful to something already on this
    /// machine. It still only opens a screen.
    Sandbox { id: i64 },
}

/// Content kinds a link may name.
///
/// A closed list rather than "whatever the API accepts": the value becomes a
/// path segment in the app's router, and a router that accepts arbitrary text
/// from a URL somebody else wrote is one route away from being interesting.
const KINDS: [&str; 8] = [
    "mod",
    "asset",
    "server",
    "serverMap",
    "article",
    "community",
    "collection",
    "user",
];

/// Parse a link, or `None`.
///
/// `None` for anything unrecognised, and the caller does nothing with it — a
/// link the app does not understand must not fall through to a default action.
pub fn parse(raw: &str) -> Option<DeepLink> {
    if raw.len() > MAX_URL {
        return None;
    }

    let url = url::Url::parse(raw).ok()?;

    if url.scheme() != SCHEME {
        return None;
    }

    /*
     * `tmc://auth` parses with `auth` as the HOST, not as a path segment, and
     * `tmc:///auth` puts it in the path. Both shapes appear in the wild —
     * different platforms' openers normalise differently — so the action is
     * taken from the host when there is one and from the first path segment
     * otherwise.
     */
    let mut segments: Vec<String> = Vec::new();

    if let Some(host) = url.host_str() {
        segments.push(host.to_ascii_lowercase());
    }

    if let Some(path) = url.path_segments() {
        segments.extend(path.filter(|s| !s.is_empty()).map(str::to_string));
    }

    let action = segments.first()?.as_str();

    match action {
        // Everything after it is ignored. A wake-up has no arguments, and
        // reading one would be reading a value an attacker chose.
        "auth" | "auth-return" | "login" => Some(DeepLink::Auth),

        "install" | "view" => {
            let kind = segments.get(1)?;
            let id = parse_id(segments.get(2)?)?;

            // Matched case-insensitively but stored in the API's own spelling,
            // so `serverMap` reaches the router as `serverMap` rather than as
            // whatever case the link happened to use.
            let kind = KINDS
                .iter()
                .find(|known| known.eq_ignore_ascii_case(kind))?;

            Some(if action == "install" {
                DeepLink::Install {
                    kind: (*kind).to_string(),
                    id,
                }
            } else {
                DeepLink::View {
                    kind: (*kind).to_string(),
                    id,
                }
            })
        }

        "sandbox" => Some(DeepLink::Sandbox {
            id: parse_id(segments.get(1)?)?,
        }),

        _ => None,
    }
}

fn parse_id(raw: &str) -> Option<i64> {
    let id: i64 = raw.parse().ok()?;

    (id > 0 && id <= MAX_ID).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_auth_link_carries_nothing() {
        for raw in [
            "tmc://auth",
            "tmc://auth/",
            "tmc:///auth",
            "tmc://auth-return",
            "tmc://auth?token=stolen",
            "tmc://auth/anything/at/all",
        ] {
            assert_eq!(parse(raw), Some(DeepLink::Auth), "{raw}");
        }
    }

    #[test]
    fn an_install_link_names_an_item() {
        assert_eq!(
            parse("tmc://install/mod/1234"),
            Some(DeepLink::Install {
                kind: "mod".into(),
                id: 1234
            })
        );

        assert_eq!(
            parse("tmc://view/asset/7"),
            Some(DeepLink::View {
                kind: "asset".into(),
                id: 7
            })
        );

        // The API's own spelling comes back, whatever case the link used.
        assert_eq!(
            parse("tmc://install/SERVERMAP/9"),
            Some(DeepLink::Install {
                kind: "serverMap".into(),
                id: 9
            })
        );
    }

    #[test]
    fn a_sandbox_link_opens_one_sandbox() {
        assert_eq!(parse("tmc://sandbox/3"), Some(DeepLink::Sandbox { id: 3 }));
    }

    /// The point of the whole module: a link may only ever name something to
    /// SHOW. Anything that looks like an instruction is refused.
    #[test]
    fn nothing_that_is_not_a_recognised_action_parses() {
        for bad in [
            // Actions that do not exist, and had better not start existing by
            // accident.
            "tmc://deploy/1",
            "tmc://run/mod/1",
            "tmc://exec/rm",
            "tmc://settings/gameDir?path=/",
            "tmc://rcon/1/exec?command=quit",
            // Another app's scheme.
            "https://example.com/install/mod/1",
            "file:///etc/passwd",
            "steam://rungameid/1",
            // Malformed.
            "tmc://",
            "tmc://install",
            "tmc://install/mod",
            "not a url",
            "",
        ] {
            assert_eq!(parse(bad), None, "{bad} should not parse");
        }
    }

    #[test]
    fn an_unknown_content_kind_is_refused() {
        // The kind becomes a router path segment, so the list is closed.
        for bad in [
            "tmc://install/../../etc/1",
            "tmc://install/settings/1",
            "tmc://install/%2e%2e/1",
            "tmc://install/mods/1",
        ] {
            assert_eq!(parse(bad), None, "{bad} should not parse");
        }
    }

    #[test]
    fn an_absurd_or_negative_id_is_refused() {
        for bad in [
            "tmc://install/mod/0",
            "tmc://install/mod/-1",
            "tmc://install/mod/99999999999999",
            "tmc://install/mod/abc",
            "tmc://install/mod/1e9",
            "tmc://sandbox/0",
        ] {
            assert_eq!(parse(bad), None, "{bad} should not parse");
        }
    }

    #[test]
    fn an_oversized_link_is_refused_before_it_is_parsed() {
        let huge = format!("tmc://install/mod/1?{}", "a".repeat(MAX_URL));

        assert_eq!(parse(&huge), None);
    }
}
