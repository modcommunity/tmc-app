//! Canonicalising a path, without the prefix Windows adds to it.
//!
//! `std::fs::canonicalize` on Windows does not return `F:\SteamLibrary`. It
//! returns `\\?\F:\SteamLibrary` — the *verbatim* form, which tells the Win32
//! layer to skip its own normalisation. That prefix is what the OS hands back
//! and it is correct; it is also wrong for nearly everything this app does with
//! the result:
//!
//!   * **It is shown to people.** A game directory is canonicalised on the way
//!     into `settings.json`, and that stored string is what the Library row,
//!     the sandbox editor, the scan dialog and every audit line print. Four
//!     punctuation marks in front of every Steam path read as a bug, because
//!     that is exactly what they look like.
//!   * **It is handed to other programs.** A verbatim path is not an ordinary
//!     path: `.` and `..` stop meaning anything, `/` stops working as a
//!     separator, and plenty of game launchers and loaders refuse one.
//!   * **It does not compare.** `\\?\F:\Games` and `F:\Games` are one directory
//!     and two strings, so any code that canonicalises one side of a comparison
//!     and not the other silently answers "different". That is the trap
//!     [`crate::plugins::jail`] sits one edit away from, since its containment
//!     check is `starts_with` between a resolved path and a joined one.
//!
//! So everything in this crate that canonicalises goes through [`canonicalize`]
//! and the prefix comes off.
//!
//! WHAT IS DELIBERATELY LEFT ALONE
//! ------------------------------
//! The prefix stays wherever removing it would change which file is named, or
//! stop one being reachable at all:
//!
//!   * a **volume GUID** — `\\?\Volume{…}`, a drive mounted without a letter —
//!     which has no shorter spelling;
//!   * a path at or past [`MAX_PATH`], where the prefix is the only reason the
//!     OS accepts it (Rust's own `std::fs` adds one back when it needs to, so a
//!     short path never has to carry one);
//!   * a component Win32 normalisation would rewrite — a trailing dot or space,
//!     a `.` or `..`, a forward slash, or a reserved device name like `NUL`.
//!     Only a verbatim path can name those, which is the whole point of it.
//!
//! The UNC case does have a plain equivalent: `\\?\UNC\server\share` is
//! `\\server\share`, and that is the spelling a person recognises and a
//! launcher accepts.
//!
//! WHY THE STRIPPING IS STRING WORK
//! -------------------------------
//! [`strip_verbatim`] takes a `&str` and is compiled and tested on every
//! platform, rather than living behind `#[cfg(windows)]` where CI would never
//! run a line of it. Only the DECISION to apply it is platform-specific — on
//! Unix a file may genuinely be called `\\?\x`, and renaming it would be this
//! module inventing a bug rather than fixing one.

use std::io;
use std::path::{Path, PathBuf};

/// The length at which a Windows path needs the verbatim prefix to work.
pub const MAX_PATH: usize = 260;

/// The prefix itself, and its UNC spelling.
const VERBATIM: &str = r"\\?\";
const VERBATIM_UNC: &str = r"\\?\UNC\";

/// MS-DOS device names, which a path component must not be.
const DEVICES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Canonicalise `path`, in the spelling the rest of the machine uses.
///
/// `std::fs::canonicalize`, with the Windows verbatim prefix removed where the
/// path has a plain equivalent. It fails for the same reasons the standard one
/// does: the path does not exist, or a component of it cannot be read.
pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
    Ok(simplify(std::fs::canonicalize(path)?))
}

/// Canonicalise, or keep what was given.
///
/// The pattern this crate reaches for most: a path that may not exist yet is
/// still the best answer available, and what it feeds is a lexical comparison
/// either way. Written once so the fallback is the same everywhere — a mix of
/// resolved and unresolved paths in one `starts_with` is the bug this module
/// exists to prevent.
pub fn canonicalize_or_keep(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();

    canonicalize(path).unwrap_or_else(|_| simplify(path.to_path_buf()))
}

/// Drop a verbatim prefix from an already-resolved path.
///
/// A no-op off Windows, and deliberately: `\\?\x` is a legal Unix filename, and
/// a path module that renamed one would be inventing a bug rather than fixing
/// one. Windows is the only platform that produces the prefix and the only one
/// where it means anything.
pub fn simplify(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }

    match path.to_str().and_then(strip_verbatim) {
        Some(plain) => PathBuf::from(plain),
        None => path,
    }
}

/// `\\?\F:\Games` → `F:\Games`, when that names the same file.
///
/// `None` means it does not — see the module header for the four cases where
/// the prefix is load-bearing rather than decorative.
pub fn strip_verbatim(text: &str) -> Option<String> {
    let plain = if let Some(rest) = text.strip_prefix(VERBATIM_UNC) {
        // `\\?\UNC\server\share\…` → `\\server\share\…`
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(VERBATIM) {
        // `\\?\F:\…` → `F:\…`. A drive letter and nothing else: everything
        // else after the prefix is a volume GUID or a device path, neither of
        // which has a shorter spelling.
        let mut chars = rest.chars();

        if !chars.next()?.is_ascii_alphabetic() || chars.next()? != ':' {
            return None;
        }

        match chars.next() {
            Some('\\') | None => rest.to_string(),
            _ => return None,
        }
    } else {
        return None;
    };

    if plain.len() >= MAX_PATH {
        return None;
    }

    /*
     * Everything below is a way for the two spellings to name DIFFERENT files.
     * Win32 normalises a plain path and does not normalise a verbatim one, so
     * a component that normalisation would rewrite has to keep its prefix —
     * a trailing dot or space is trimmed, `/` becomes a separator, `.` and
     * `..` are resolved, and a device name stops being a file at all.
     *
     * None of these can come out of a real `canonicalize`, which is what makes
     * the check cheap: it costs one pass and closes the door on a path that
     * reached `simplify` from somewhere else.
     */
    if plain.contains('/') {
        return None;
    }

    for part in plain.split('\\') {
        if part == "." || part == ".." || part.ends_with('.') || part.ends_with(' ') {
            return None;
        }

        let stem = part.split('.').next().unwrap_or(part);

        if DEVICES.iter().any(|dev| stem.eq_ignore_ascii_case(dev)) {
            return None;
        }
    }

    Some(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalising_an_existing_directory_agrees_with_std() {
        let tmp = std::env::temp_dir();

        let ours = canonicalize(&tmp).expect("canonical");
        let theirs = std::fs::canonicalize(&tmp).expect("canonical");

        assert_eq!(simplify(theirs), ours);
        assert!(!ours.to_string_lossy().starts_with(VERBATIM));
    }

    #[test]
    fn a_missing_path_keeps_what_it_was_given() {
        let ghost = std::env::temp_dir().join("tmc-canon-not-here");

        assert!(canonicalize(&ghost).is_err());
        assert_eq!(canonicalize_or_keep(&ghost), ghost);
    }

    /// The property every caller depends on: resolving twice changes nothing,
    /// so a stored path compares equal to a freshly resolved one.
    #[test]
    fn simplifying_is_idempotent() {
        let once = canonicalize(std::env::temp_dir()).expect("canonical");

        assert_eq!(simplify(once.clone()), once);
    }

    /// Unix filenames may contain backslashes, and one that looks like a
    /// Windows prefix is still just a filename.
    #[cfg(not(windows))]
    #[test]
    fn nothing_is_stripped_off_windows() {
        for raw in [r"\\?\F:\Games", "/home/someone/Games/Rust"] {
            let path = PathBuf::from(raw);

            assert_eq!(simplify(path.clone()), path);
        }
    }

    // ---------------------------------------------------------------------
    // `strip_verbatim` is pure string work, so these run on every platform —
    // including the CI runners that will never execute the Windows branch.
    // ---------------------------------------------------------------------

    #[test]
    fn a_verbatim_disk_path_loses_its_prefix() {
        assert_eq!(
            strip_verbatim(r"\\?\F:\SteamLibrary\steamapps\common\GarrysMod").as_deref(),
            Some(r"F:\SteamLibrary\steamapps\common\GarrysMod")
        );

        // A bare drive, which is what a library on the root of a disk gives.
        assert_eq!(strip_verbatim(r"\\?\F:\").as_deref(), Some(r"F:\"));
    }

    #[test]
    fn a_verbatim_unc_path_becomes_an_ordinary_share() {
        assert_eq!(
            strip_verbatim(r"\\?\UNC\nas\games\Rust").as_deref(),
            Some(r"\\nas\games\Rust")
        );
    }

    #[test]
    fn an_ordinary_path_is_not_touched() {
        for raw in [r"F:\Games", r"\\nas\games", "/home/someone/Games", ""] {
            assert_eq!(strip_verbatim(raw), None, "{raw}");
        }
    }

    /// A volume with no drive letter has no shorter spelling, and neither has
    /// anything else that is verbatim for a reason.
    #[test]
    fn a_path_with_no_plain_equivalent_keeps_the_prefix() {
        for raw in [
            r"\\?\Volume{d7d7e4c2-0000-0000-0000-100000000000}\Games",
            r"\\?\GLOBALROOT\Device\HarddiskVolume2",
            r"\\?\F",
            r"\\?\F:junction",
        ] {
            assert_eq!(strip_verbatim(raw), None, "{raw}");
        }
    }

    /// Past MAX_PATH the prefix is the only reason the OS accepts the path, so
    /// a tidier version of it is one that stops working.
    #[test]
    fn a_long_path_keeps_the_prefix_that_makes_it_legal() {
        let long = format!(r"\\?\F:\{}", "a".repeat(300));

        assert_eq!(strip_verbatim(&long), None);
    }

    /// The cases where the two spellings name different files. Win32
    /// normalises the plain one and not the verbatim one.
    #[test]
    fn a_component_normalisation_would_rewrite_keeps_the_prefix() {
        for raw in [
            r"\\?\F:\Games\trailing.",
            r"\\?\F:\Games\trailing ",
            r"\\?\F:\Games\..\Other",
            r"\\?\F:\Games\.\Other",
            r"\\?\F:\Games/Other",
            r"\\?\F:\Games\NUL",
            r"\\?\F:\Games\nul.txt",
            r"\\?\F:\Games\COM1",
        ] {
            assert_eq!(strip_verbatim(raw), None, "{raw}");
        }
    }
}
