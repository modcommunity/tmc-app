//! Notification polling, minus the window.
//!
//! The tray build polls `GET /api/app/v1/notifications` and shows each new item
//! as a card in the corner of the screen. Everything that can be decided
//! without a window lives here so it is testable on a machine with no display:
//! parsing the response, bounding what the site sent, deciding which items are
//! NEW, and choosing the card's look.
//!
//! What the site sends is displayed to the user as text and nothing else. It
//! never becomes markup, a command or a path; the one thing it can influence is
//! a URL, and [`safe_open_url`] decides whether that may be opened.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most items one poll may ask for or act on.
pub const POLL_LIMIT: usize = 10;
/// More new items than this in one poll collapse into a single summary card,
/// so a returning user is not buried under a stack of windows.
pub const MAX_CARDS_PER_POLL: usize = 3;

const TITLE_MAX: usize = 90;
const BODY_MAX: usize = 220;

/// One notification as the card shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub id: String,
    /// The site's category (`FRIEND`, `PARTY`, …), upper-case.
    pub kind: String,
    pub title: String,
    pub body: String,
    /// Where clicking the card goes, when the site gave a usable link.
    pub url: Option<String>,
    /// ISO-8601 creation time. Compared as text, which orders correctly for
    /// the site's fixed-width UTC timestamps.
    pub created_at: String,
    pub actor: Option<String>,
}

/// A card's look: an accent colour and a short glyph. Unknown kinds get the
/// neutral one rather than an error, because the site may add a category
/// before this build learns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Style {
    pub accent: &'static str,
    pub glyph: &'static str,
    pub label: &'static str,
}

pub fn style_for(kind: &str) -> Style {
    let (accent, glyph, label) = match kind {
        "FRIEND" | "FOLLOW" => ("#5fd38d", "👋", "Friends"),
        "PARTY" | "SERVER_INVITE" => ("#ffc83d", "🎮", "Play"),
        "MESSAGE" => ("#4da3ff", "💬", "Message"),
        "MENTION" | "COMMENT" | "REVIEW" => ("#b58cff", "💭", "Community"),
        "RELEASE" => ("#e4572e", "🚀", "Release"),
        "CREDIT" | "POINTS" => ("#f5b942", "⭐", "Points"),
        "MODERATION" => ("#ff6b6b", "🛡️", "Moderation"),
        "GROUP" => ("#4dd0c8", "👥", "Group"),
        "STATUS" => ("#9aa7b4", "📡", "Status"),
        _ => ("#e4572e", "🔔", "GameSpice"),
    };

    Style { accent, glyph, label }
}

/// Strip control characters, collapse whitespace and bound the length. The
/// result is plain text for a text node.
pub fn clean(raw: &str, max_chars: usize) -> String {
    let joined = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>();

    let squashed = joined.split_whitespace().collect::<Vec<_>>().join(" ");

    if squashed.chars().count() <= max_chars {
        return squashed;
    }

    let mut cut: String = squashed.chars().take(max_chars.saturating_sub(1)).collect();

    cut.push('…');

    cut
}

/// Parse the `data` of a notifications response, newest first as sent.
/// Unreadable items are skipped rather than failing the poll: one odd row must
/// not silence every notification after it.
pub fn parse(data: &Value) -> (u64, Vec<Notice>) {
    let unread = data.get("unread").and_then(Value::as_u64).unwrap_or(0);

    let items = data
        .get("items")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(parse_one).take(POLL_LIMIT).collect())
        .unwrap_or_default();

    (unread, items)
}

fn parse_one(row: &Value) -> Option<Notice> {
    let id = match row.get("id")? {
        Value::String(s) if !s.is_empty() => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };

    let created_at = row.get("createdAt")?.as_str()?.to_string();
    let title = clean(row.get("title")?.as_str()?, TITLE_MAX);

    if title.is_empty() {
        return None;
    }

    // An already-read item is never worth a card.
    if row.get("read").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }

    let text = |key: &str, max| {
        row.get(key)
            .and_then(Value::as_str)
            .map(|s| clean(s, max))
            .filter(|s| !s.is_empty())
    };

    Some(Notice {
        id,
        kind: row
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("SYSTEM")
            .to_ascii_uppercase(),
        title,
        body: text("body", BODY_MAX).unwrap_or_default(),
        url: text("url", 2048),
        created_at,
        actor: text("actorName", 60),
    })
}

/// Remembers how far the user has been told.
///
/// The first poll after launch only sets the cursor: showing every unread item
/// from the last week as a card the moment the app starts is a notification
/// storm, and the bell in the app already holds them.
#[derive(Debug, Default)]
pub struct Cursor {
    seen_through: Option<String>,
    seen_ids: Vec<String>,
}

impl Cursor {
    /// The `since` to send, once there is one.
    pub fn since(&self) -> Option<&str> {
        self.seen_through.as_deref()
    }

    /// Forget everything — a different account or a sign-out.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The items to announce, oldest first. Advances the cursor either way.
    pub fn advance(&mut self, items: Vec<Notice>) -> Vec<Notice> {
        let first = self.seen_through.is_none();

        let newest = items.iter().map(|n| n.created_at.as_str()).max();

        let mut fresh: Vec<Notice> = items
            .iter()
            .filter(|n| !self.seen_ids.contains(&n.id))
            .filter(|n| self.seen_through.as_deref().map_or(true, |t| n.created_at.as_str() >= t))
            .cloned()
            .collect();

        if let Some(newest) = newest {
            if self.seen_through.as_deref().map_or(true, |t| newest > t) {
                self.seen_through = Some(newest.to_string());
            }
        } else if first {
            // Nothing unread at all: the baseline is "now", which an empty
            // string cannot express, so use a sentinel that sorts first.
            self.seen_through = Some(String::new());
        }

        for n in &items {
            if !self.seen_ids.contains(&n.id) {
                self.seen_ids.push(n.id.clone());
            }
        }

        // Bounded: the ids only exist to absorb the `>=` overlap above.
        if self.seen_ids.len() > 200 {
            let excess = self.seen_ids.len() - 200;

            self.seen_ids.drain(..excess);
        }

        if first {
            return Vec::new();
        }

        fresh.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        fresh
    }
}

/// A site path or an https URL on the API's own host may be opened from a card.
/// Anything else is dropped: a card is the one place text from the network can
/// become an action, and the action must be one the site could have linked to.
pub fn safe_open_url(raw: &str, site_base: &str) -> Option<String> {
    let base = url_origin(site_base)?;

    if raw.starts_with('/') && !raw.starts_with("//") && !raw.contains('\\') {
        return Some(format!("{base}{raw}"));
    }

    let origin = url_origin(raw)?;

    (origin == base && raw.starts_with("https://")).then(|| raw.to_string())
}

fn url_origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;

    if scheme != "https" && scheme != "http" {
        return None;
    }

    let host = rest.split(['/', '?', '#']).next()?;

    if host.is_empty() || host.contains('@') {
        return None;
    }

    Some(format!("{scheme}://{}", host.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn n(id: &str, at: &str) -> Notice {
        Notice {
            id: id.into(),
            kind: "FRIEND".into(),
            title: "t".into(),
            body: String::new(),
            url: None,
            created_at: at.into(),
            actor: None,
        }
    }

    #[test]
    fn first_poll_is_silent_then_only_new_items_show() {
        let mut c = Cursor::default();

        assert!(c.advance(vec![n("1", "2026-01-01T00:00:01Z")]).is_empty());
        assert_eq!(c.since(), Some("2026-01-01T00:00:01Z"));

        // The overlapping item is not repeated; the new one is.
        let out = c.advance(vec![n("2", "2026-01-01T00:00:05Z"), n("1", "2026-01-01T00:00:01Z")]);

        assert_eq!(out.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["2"]);
    }

    #[test]
    fn empty_first_poll_still_sets_a_baseline() {
        let mut c = Cursor::default();

        assert!(c.advance(vec![]).is_empty());
        assert_eq!(c.since(), Some(""));
        assert_eq!(c.advance(vec![n("9", "2026-01-02T00:00:00Z")]).len(), 1);
    }

    #[test]
    fn announced_oldest_first() {
        let mut c = Cursor::default();

        c.advance(vec![]);

        let out = c.advance(vec![n("b", "2026-01-01T00:00:09Z"), n("a", "2026-01-01T00:00:03Z")]);

        assert_eq!(out[0].id, "a");
    }

    #[test]
    fn parse_skips_read_and_broken_rows() {
        let (unread, items) = parse(&json!({
            "unread": 2,
            "items": [
                {"id": "1", "type": "party", "title": "Join us\n now", "body": "x", "url": "/p", "read": false, "createdAt": "2026-01-01T00:00:00Z"},
                {"id": "2", "type": "FRIEND", "title": "seen", "read": true, "createdAt": "2026-01-01T00:00:00Z"},
                {"title": "no id", "createdAt": "2026-01-01T00:00:00Z"},
                "junk"
            ]
        }));

        assert_eq!(unread, 2);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "PARTY");
        assert_eq!(items[0].title, "Join us now");
    }

    #[test]
    fn clean_bounds_and_strips() {
        assert_eq!(clean("a\u{7}b\tc", 10), "a b c");
        assert_eq!(clean(&"x".repeat(500), 10).chars().count(), 10);
    }

    #[test]
    fn only_the_sites_own_links_open() {
        let base = "https://moddingcommunity.com";

        assert_eq!(
            safe_open_url("/party/1", base).as_deref(),
            Some("https://moddingcommunity.com/party/1")
        );
        assert!(safe_open_url("https://moddingcommunity.com/x", base).is_some());
        assert!(safe_open_url("https://evil.example/x", base).is_none());
        assert!(safe_open_url("//evil.example/x", base).is_none());
        assert!(safe_open_url("javascript:alert(1)", base).is_none());
        assert!(safe_open_url("file:///etc/passwd", base).is_none());
        assert!(safe_open_url("https://moddingcommunity.com@evil.example/", base).is_none());
    }

    #[test]
    fn unknown_kinds_get_the_default_look() {
        assert_eq!(style_for("NEW_THING").label, "GameSpice");
    }
}
