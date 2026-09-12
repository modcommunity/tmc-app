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

    /// `tmc://play/<host>:<port>` — open a join screen for that address.
    ///
    /// The link the website's Join button produces (`PLAY_URI_DEFAULT`), and
    /// the one place in this enum where the payload is not an id of ours. It is
    /// STILL only a screen: the app looks the address up, shows what is running
    /// there, and puts a Join button on it. It does not start a game.
    ///
    /// That distinction is the whole module's rule and it is load-bearing here
    /// rather than theoretical, because this is the link an ordinary web page
    /// can navigate to without a click. An auto-joining version would be a
    /// remote primitive for making somebody's machine connect to an address a
    /// stranger chose — which is a small harm on its own and is exactly the
    /// shape of the thing this app spends four hundred lines avoiding
    /// elsewhere.
    ///
    /// The port is optional: `tmc://play/example.com` is a perfectly good way
    /// to say "this box", and the game's own default port is a thing the join
    /// screen can fill in.
    Play { host: String, port: Option<u16> },

    /// `tmc://play/app/<slug-or-id>` — open one game's launch screen.
    ///
    /// The serverless half. A slug rather than only an id because that is what
    /// the website's `{app}` token substitutes, and because a link somebody
    /// writes by hand is written with a name in it.
    PlayApp { app: String },
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

        "play" => {
            /*
             * EXACTLY the two shapes, with nothing after them.
             *
             * A trailing segment is ignored by `auth`, where that is right — a
             * wake-up has no arguments and reading one would be reading a value
             * an attacker chose. Here it is wrong for the opposite reason: this
             * action HAS a grammar, and accepting `tmc://play/host:1/join`
             * today is how `/join` quietly becomes meaningful the first time
             * somebody adds a segment to the match below.
             */
            let first = segments.get(1)?;

            let expected = if first.eq_ignore_ascii_case("app") {
                3
            } else {
                2
            };

            if segments.len() != expected {
                return None;
            }

            /*
             * `play/app/<x>` is the serverless shape, and is checked before the
             * address one so that a game whose slug happens to be `app` cannot
             * be reached as an address. The alternative ordering would make
             * `tmc://play/app` parse as a hostname called "app".
             */
            if first.eq_ignore_ascii_case("app") {
                let app = segments.get(2)?;

                return parse_app_ref(app).map(|app| DeepLink::PlayApp { app });
            }

            let (host, port) = split_address(first)?;

            Some(DeepLink::Play { host, port })
        }

        _ => None,
    }
}

/// Split `host:port`, `host`, or `[v6]:port` into its two halves.
///
/// Written out rather than handed to `SocketAddr::from_str`, for two reasons
/// that both matter here: the host is very often a NAME and not an address, and
/// the port is optional. What this does check is that the host could be one —
/// the value becomes a lookup and then a screen, and a "hostname" containing a
/// slash, a space or a control character is not a typo somebody made.
fn split_address(raw: &str) -> Option<(String, Option<u16>)> {
    // A bracketed IPv6 literal keeps its brackets: `[::1]` is how the address
    // is written everywhere a port might follow it, and splitting on the last
    // colon without them would take `::1` apart in the middle.
    let (host, port) = if let Some(rest) = raw.strip_prefix('[') {
        let close = rest.find(']')?;
        let host = format!("[{}]", &rest[..close]);

        match rest[close + 1..].strip_prefix(':') {
            Some(port) => (host, Some(port)),
            None if rest[close + 1..].is_empty() => (host, None),
            None => return None,
        }
    } else {
        match raw.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), Some(port)),
            None => (raw.to_string(), None),
        }
    };

    if !is_hostish(&host) {
        return None;
    }

    let port = match port {
        Some(raw) => {
            let parsed: u16 = raw.parse().ok()?;

            // Port 0 is "let the OS pick", which is not something a client can
            // connect to and is the value an empty field parses to.
            (parsed > 0).then_some(parsed)?.into()
        }
        None => None,
    };

    Some((host, port))
}

/// Whether this could be a hostname or an IP literal.
///
/// Deliberately not a full RFC check: the value is going to a lookup that will
/// refuse it if it is not real, and the job here is to keep anything that is
/// obviously not an address out of a router path and out of a request.
fn is_hostish(host: &str) -> bool {
    let inner = host.strip_prefix('[').and_then(|h| h.strip_suffix(']'));

    // An IPv6 literal is hex groups and colons, and nothing else.
    if let Some(inner) = inner {
        return !inner.is_empty()
            && inner.len() <= 45
            && inner
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
    }

    !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// A slug or a numeric id, as the `{app}` token substitutes one.
///
/// The value becomes a path segment in the app's router and a query parameter
/// in a request, so the character set is closed for the same reason [`KINDS`]
/// is.
fn parse_app_ref(raw: &str) -> Option<String> {
    let trimmed = raw.trim();

    let ok = !trimmed.is_empty()
        && trimmed.len() <= 120
        && trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');

    ok.then(|| trimmed.to_ascii_lowercase())
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

    #[test]
    fn a_play_link_names_an_address_and_only_opens_a_screen() {
        assert_eq!(
            parse("tmc://play/play.example.com:6064"),
            Some(DeepLink::Play {
                host: "play.example.com".into(),
                port: Some(6064)
            })
        );

        // No port is a legitimate way to name a box.
        assert_eq!(
            parse("tmc://play/192.0.2.10"),
            Some(DeepLink::Play {
                host: "192.0.2.10".into(),
                port: None
            })
        );

        // A bracketed v6 literal keeps its brackets and its colons.
        assert_eq!(
            parse("tmc://play/[2001:db8::1]:27015"),
            Some(DeepLink::Play {
                host: "[2001:db8::1]".into(),
                port: Some(27015)
            })
        );

        /*
         * `..` never reaches the match: the URL parser normalises the path
         * before this module sees it, so this is a link naming a host called
         * `etc`. Pinned because the result LOOKS like a traversal that got
         * through, and the next person to read it should not have to re-derive
         * why it did not — what comes out is a hostname, it is only ever used
         * as one, and no such host resolves.
         */
        assert_eq!(
            parse("tmc://play/../etc"),
            Some(DeepLink::Play {
                host: "etc".into(),
                port: None
            })
        );
    }

    #[test]
    fn a_play_app_link_names_a_game() {
        assert_eq!(
            parse("tmc://play/app/hungario"),
            Some(DeepLink::PlayApp {
                app: "hungario".into()
            })
        );

        // The id form, which is what `{app}` falls back to.
        assert_eq!(
            parse("tmc://play/app/42"),
            Some(DeepLink::PlayApp { app: "42".into() })
        );
    }

    #[test]
    fn a_play_link_that_is_not_an_address_is_refused() {
        for bad in [
            // The shape an empty `{host}`/`{port}` substitution used to
            // produce. The website declines to build it now; this is the other
            // half of that fix, on the side that would have to act on it.
            "tmc://play/:",
            "tmc://play/:6064",
            "tmc://play/host:0",
            "tmc://play/host:99999",
            "tmc://play/host:abc",
            "tmc://play/two%20words",
            "tmc://play/a/b",
            "tmc://play",
            "tmc://play/app",
            // `tmc://play/app/../x` is absent on purpose: the URL parser
            // normalises it to `tmc://play/x` before this module sees it, so it
            // is a link naming a host — see the test above.
            "tmc://play/[2001:db8::1",
        ] {
            assert_eq!(parse(bad), None, "{bad} should not parse");
        }
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
            // `play` SHOWS a join screen. These would be it doing something.
            "tmc://play/host:1/join",
            "tmc://launch/host:1",
            "tmc://connect/host:1",
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
