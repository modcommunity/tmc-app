//! Valve's KeyValues text format, in as little code as reading it needs.
//!
//! Steam stores its library list (`libraryfolders.vdf`) and one manifest per
//! installed game (`appmanifest_<id>.acf`) in this format:
//!
//! ```text
//! "AppState"
//! {
//!     "appid"       "271590"
//!     "name"        "Grand Theft Auto V"
//!     "installdir"  "Grand Theft Auto V"
//!     "UserConfig"
//!     {
//!         "language"  "english"
//!     }
//! }
//! ```
//!
//! Quoted strings and braces, and that is the whole grammar this needs. The
//! format has more in it — `#include`, conditionals like `[$WIN32]`, unquoted
//! tokens, C-style escapes — and none of it appears in the two files this
//! reads. Implementing the rest would be more code, more to get wrong, and
//! would still not make this a general VDF library.
//!
//! WHY IT IS WRITTEN DEFENSIVELY ANYWAY
//! -----------------------------------
//! These files are not hostile — they belong to a program the user installed —
//! but they are also not ours, they can be truncated by a crashed Steam client,
//! and this parser runs at launch on every platform. So it is bounded on every
//! axis (size, depth, token count) and returns `None` rather than panicking or
//! looping, exactly like the protocol parsers under `net/query/`.

use std::collections::BTreeMap;

/// Cap on a file this will parse. The largest real `libraryfolders.vdf` is a
/// few kilobytes; the largest `appmanifest` is tens.
pub const MAX_BYTES: usize = 4 * 1024 * 1024;

/// Cap on nesting. Real files reach three.
const MAX_DEPTH: usize = 24;

/// Cap on tokens, so a file of nothing but quotes terminates.
const MAX_TOKENS: usize = 200_000;

/// A parsed node: either a string or a block of named children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Map(BTreeMap<String, Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            Self::Map(_) => None,
        }
    }

    pub fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Self::Map(m) => Some(m),
            Self::Str(_) => None,
        }
    }

    /// Look up a child, case-insensitively.
    ///
    /// Steam is not consistent about case — `AppState` and `appid` sit in the
    /// same file, and `libraryfolders` has been written both ways across client
    /// versions. Matching exactly means a parser that works until Valve ships a
    /// release that capitalises something.
    pub fn get(&self, key: &str) -> Option<&Value> {
        let map = self.as_map()?;

        map.get(key).or_else(|| {
            map.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v)
        })
    }

    /// A string child.
    pub fn str_at(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }
}

/// Parse a whole document.
///
/// The result is the TOP-LEVEL map, not the single root node the file usually
/// has — a file may legitimately hold several root blocks, and callers want to
/// address them by name either way.
pub fn parse(raw: &str) -> Option<Value> {
    if raw.len() > MAX_BYTES {
        return None;
    }

    let mut cursor = Cursor {
        bytes: raw.as_bytes(),
        pos: 0,
        tokens: 0,
    };

    let map = cursor.parse_block(0)?;

    Some(Value::Map(map))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    tokens: usize,
}

impl Cursor<'_> {
    /// Parse `"key" value` pairs until a `}` or the end of input.
    fn parse_block(&mut self, depth: usize) -> Option<BTreeMap<String, Value>> {
        if depth > MAX_DEPTH {
            return None;
        }

        let mut out = BTreeMap::new();

        loop {
            self.skip_trivia();

            if self.pos >= self.bytes.len() {
                return Some(out);
            }

            if self.bytes[self.pos] == b'}' {
                self.pos += 1;

                return Some(out);
            }

            let key = self.read_token()?;

            self.skip_trivia();

            if self.pos >= self.bytes.len() {
                // A key with no value at the end of a truncated file. Keeping
                // what was read beats discarding the whole document.
                return Some(out);
            }

            let value = if self.bytes[self.pos] == b'{' {
                self.pos += 1;

                Value::Map(self.parse_block(depth + 1)?)
            } else {
                Value::Str(self.read_token()?)
            };

            out.insert(key, value);
        }
    }

    /// Whitespace and `//` comments.
    fn skip_trivia(&mut self) {
        loop {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }

            if self.pos + 1 < self.bytes.len()
                && self.bytes[self.pos] == b'/'
                && self.bytes[self.pos + 1] == b'/'
            {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }

                continue;
            }

            return;
        }
    }

    /// A quoted string, or a bare word up to the next whitespace or brace.
    fn read_token(&mut self) -> Option<String> {
        self.tokens += 1;

        if self.tokens > MAX_TOKENS {
            return None;
        }

        if self.pos >= self.bytes.len() {
            return None;
        }

        if self.bytes[self.pos] != b'"' {
            let start = self.pos;

            while self.pos < self.bytes.len()
                && !self.bytes[self.pos].is_ascii_whitespace()
                && self.bytes[self.pos] != b'{'
                && self.bytes[self.pos] != b'}'
            {
                self.pos += 1;
            }

            if self.pos == start {
                // Neither a token nor whitespace — a stray brace. Consume it so
                // the loop cannot spin.
                self.pos += 1;

                return Some(String::new());
            }

            return Some(String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned());
        }

        self.pos += 1;

        let mut out = Vec::new();

        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'"' => {
                    self.pos += 1;

                    return Some(String::from_utf8_lossy(&out).into_owned());
                }
                b'\\' if self.pos + 1 < self.bytes.len() => {
                    /*
                     * Windows paths are the reason this matters:
                     * `"path" "D:\\SteamLibrary"` is how Steam writes one, and
                     * a parser that keeps the backslashes doubled produces a
                     * path that does not exist.
                     */
                    let escaped = self.bytes[self.pos + 1];

                    out.push(match escaped {
                        b'n' => b'\n',
                        b't' => b'\t',
                        other => other,
                    });

                    self.pos += 2;
                }
                byte => {
                    out.push(byte);
                    self.pos += 1;
                }
            }
        }

        // Unterminated. Return what there was — a truncated manifest should
        // yield a partial record, not nothing.
        Some(String::from_utf8_lossy(&out).into_owned())
    }
}

/// Read and parse a file, or `None`.
pub fn parse_file(path: &std::path::Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;

    if meta.len() as usize > MAX_BYTES {
        return None;
    }

    parse(&std::fs::read_to_string(path).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY_FOLDERS: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/player/.local/share/Steam"
		"label"		""
		"contentid"		"123"
		"apps"
		{
			"271590"		"79906516045"
			"730"		"36847105136"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps"
		{
			"252490"		"10945158613"
		}
	}
}
"#;

    #[test]
    fn a_library_list_yields_its_paths() {
        let parsed = parse(LIBRARY_FOLDERS).expect("parsed");

        let folders = parsed.get("libraryfolders").expect("root");

        assert_eq!(
            folders.get("0").and_then(|f| f.str_at("path")),
            Some("/home/player/.local/share/Steam")
        );

        // The escaped backslashes have to come back as single ones, or the
        // path names a directory that does not exist.
        assert_eq!(
            folders.get("1").and_then(|f| f.str_at("path")),
            Some("D:\\SteamLibrary")
        );

        let apps = folders
            .get("0")
            .and_then(|f| f.get("apps"))
            .and_then(Value::as_map)
            .expect("apps");

        assert!(apps.contains_key("271590"));
    }

    #[test]
    fn lookups_ignore_case_because_steam_does() {
        let parsed = parse(r#""AppState" { "AppID" "271590" }"#).expect("parsed");

        assert_eq!(
            parsed.get("appstate").and_then(|a| a.str_at("appid")),
            Some("271590")
        );
    }

    #[test]
    fn comments_and_odd_whitespace_are_skipped() {
        let parsed = parse(
            r#"
            // a comment
            "root"
            {
                "a"   "1" // trailing
                "b"   "2"
            }
            "#,
        )
        .expect("parsed");

        let root = parsed.get("root").expect("root");

        assert_eq!(root.str_at("a"), Some("1"));
        assert_eq!(root.str_at("b"), Some("2"));
    }

    /// A Steam client killed mid-write leaves one of these half-finished. The
    /// parser must produce what it can rather than nothing, and must never hang.
    #[test]
    fn every_truncation_of_a_real_file_parses_or_declines() {
        for cut in 0..LIBRARY_FOLDERS.len() {
            let prefix = &LIBRARY_FOLDERS[..cut];

            if !prefix.is_char_boundary(cut) {
                continue;
            }

            // The contract is only "returns, does not panic, does not loop".
            let _ = parse(prefix);
        }
    }

    #[test]
    fn pathological_input_terminates() {
        for bad in [
            "{".repeat(1000).as_str(),
            "}".repeat(1000).as_str(),
            "\"".repeat(1000).as_str(),
            "\"a\"".repeat(1000).as_str(),
        ] {
            let _ = parse(bad);
        }

        // Deeper than the depth cap: declined, not overflowed.
        let deep = format!("\"a\" {{{}", "\"b\" {".repeat(200));

        assert!(parse(&deep).is_none());
    }

    #[test]
    fn an_oversized_document_is_declined_outright() {
        let huge = "a".repeat(MAX_BYTES + 1);

        assert!(parse(&huge).is_none());
    }
}
