//! The filesystem primitives a deployment strategy is built from, and the
//! platform facts that decide which strategies can work at all.
//!
//! Everything here is deliberately thin — one syscall and the reason it might
//! fail. The interesting decisions (what to link, in what order, what to do
//! when it fails) belong to [`super::engine`]; this module only has to be
//! right about five awkward platform details:
//!
//!   * **A hard link cannot cross a volume.** Not "should not" — the kernel
//!     refuses, and the error it returns (`EXDEV`, or `ERROR_NOT_SAME_DEVICE`)
//!     is indistinguishable from half a dozen others by the time it reaches a
//!     user. [`same_volume`] answers the question BEFORE the attempt so the
//!     message can say "your staging folder is on D: and the game is on C:".
//!   * **A symlink needs a privilege on Windows** unless Developer Mode is on.
//!     `ERROR_PRIVILEGE_NOT_HELD` (1314) is the one error that means "try a
//!     different strategy" rather than "something is broken".
//!   * **`std::fs::metadata` follows links; `symlink_metadata` does not.**
//!     Every check in a purge path has to use the second one, or a symlink
//!     pointing at a real file reports itself as a real file.
//!   * **A hard link is not distinguishable from the original.** There is no
//!     "is this a hard link" question to ask — both names are equally real.
//!     The only usable test is whether two paths name the SAME inode, which is
//!     [`same_file`].
//!   * **Windows has no inode.** `same_file` therefore degrades to a
//!     length-and-mtime comparison there, which is weaker and is documented at
//!     the call site rather than hidden here.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{AppError, AppResult};

/// How one file was put into the game directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    /// A second directory entry for the same inode. Costs nothing, is
    /// indistinguishable from a real file to the game and to every external
    /// tool, and cannot cross a volume.
    Hard,
    /// A pointer. Crosses volumes freely; a few anti-cheats and a few very old
    /// engines refuse to open one.
    Symbolic,
    /// A real copy. Always works, costs the bytes.
    Copy,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hard => "hard",
            Self::Symbolic => "symbolic",
            Self::Copy => "copy",
        }
    }
}

/// Why a link attempt failed, in terms a caller can act on.
///
/// The whole point of this enum is [`LinkFailure::Unsupported`]: it is the one
/// outcome that means "this machine cannot do it this way, try another way",
/// and separating it from a genuine IO error is what lets the engine fall back
/// automatically instead of failing a deploy that had two other options.
#[derive(Debug)]
pub enum LinkFailure {
    /// The platform or the volume refuses this kind of link, for a reason no
    /// retry will change. Carries a sentence for the user.
    Unsupported(String),
    /// Something else went wrong — permissions on this particular file, a full
    /// disk, a path that vanished.
    Io(std::io::Error),
}

impl LinkFailure {
    pub fn message(&self) -> String {
        match self {
            Self::Unsupported(why) => why.clone(),
            Self::Io(e) => e.to_string(),
        }
    }

    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported(_))
    }
}

impl From<LinkFailure> for AppError {
    fn from(f: LinkFailure) -> Self {
        match f {
            LinkFailure::Unsupported(why) => AppError::invalid(why),
            LinkFailure::Io(e) => AppError::internal(format!("link: {e}")),
        }
    }
}

// --------------------------------------------------------------- Volumes

/// Are these two paths on the same volume?
///
/// Answered from the deepest ANCESTOR of each that exists, because the whole
/// reason to ask is that one of them is about to be created.
///
/// On Unix this is the `st_dev` comparison the kernel itself uses, so it is
/// exact — including the cases a path-prefix comparison gets wrong, like a
/// bind mount or a separate `/home` partition.
///
/// On Windows there is no `st_dev` without a `winapi` dependency this crate
/// does not carry, so it compares the path PREFIX — the drive letter or the
/// UNC share. That is right for the ordinary `C:` vs `D:` case, which is the
/// case that matters, and wrong only for a mounted volume folder (a rare
/// configuration that will surface as a failed link and a fallback rather than
/// as a silent wrong answer).
pub fn same_volume(a: &Path, b: &Path) -> AppResult<bool> {
    let a = existing_ancestor(a)
        .ok_or_else(|| AppError::invalid(format!("{} is not reachable.", a.display())))?;
    let b = existing_ancestor(b)
        .ok_or_else(|| AppError::invalid(format!("{} is not reachable.", b.display())))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let da = std::fs::metadata(&a)?.dev();
        let db = std::fs::metadata(&b)?.dev();

        Ok(da == db)
    }

    #[cfg(not(unix))]
    {
        Ok(volume_prefix(&a) == volume_prefix(&b))
    }
}

#[cfg(not(unix))]
fn volume_prefix(path: &Path) -> Option<String> {
    path.components().next().and_then(|c| match c {
        std::path::Component::Prefix(p) => {
            Some(p.as_os_str().to_string_lossy().to_ascii_lowercase())
        }
        _ => None,
    })
}

/// The deepest ancestor of `path` that exists, `path` itself included.
pub fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut cursor = path;

    loop {
        if cursor.exists() {
            return Some(cursor.to_path_buf());
        }

        match cursor.parent() {
            Some(parent) if parent != cursor => cursor = parent,
            _ => return None,
        }
    }
}

// ----------------------------------------------------------------- Links

/// `ERROR_PRIVILEGE_NOT_HELD`. Windows refuses a symlink to a non-elevated
/// process unless Developer Mode is enabled, and this is how it says so.
#[cfg(windows)]
const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;

/// Create a hard link at `dst` pointing at the same data as `src`.
pub fn hard_link(src: &Path, dst: &Path) -> Result<(), LinkFailure> {
    match std::fs::hard_link(src, dst) {
        Ok(()) => Ok(()),
        Err(e) => Err(classify_hard(e, src, dst)),
    }
}

fn classify_hard(e: std::io::Error, src: &Path, dst: &Path) -> LinkFailure {
    // `CrossesDevices` is stable only on recent toolchains, so the raw code is
    // what is actually matched. 18 is EXDEV on Linux and macOS;
    // ERROR_NOT_SAME_DEVICE is 17 on Windows.
    let cross = matches!(e.raw_os_error(), Some(18)) && cfg!(unix)
        || matches!(e.raw_os_error(), Some(17)) && cfg!(windows);

    if cross {
        return LinkFailure::Unsupported(format!(
            "{} and {} are on different drives, and a hard link cannot span two drives.",
            src.display(),
            dst.display()
        ));
    }

    // A filesystem that has no hard links at all — exFAT, most network shares,
    // and every FUSE mount that did not implement `link`.
    if matches!(e.kind(), std::io::ErrorKind::Unsupported) {
        return LinkFailure::Unsupported(
            "The filesystem holding the game does not support hard links.".into(),
        );
    }

    LinkFailure::Io(e)
}

/// Create a symbolic link at `dst` pointing at `src`.
///
/// `src` is stored ABSOLUTE. A relative link would survive the staging folder
/// being moved, which sounds like an advantage until the game directory is the
/// thing that moves and every link in it silently starts resolving against a
/// path that now holds somebody else's files.
pub fn symlink_file(src: &Path, dst: &Path) -> Result<(), LinkFailure> {
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(src, dst);

    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_file(src, dst);

    #[cfg(not(any(unix, windows)))]
    let result: std::io::Result<()> = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "symlinks are not supported on this platform",
    ));

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            #[cfg(windows)]
            if e.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD) {
                return Err(LinkFailure::Unsupported(
                    "Windows will not let this app create symbolic links. Turn on Developer \
                     Mode in Windows Settings, or use the hard-link strategy instead."
                        .into(),
                ));
            }

            if matches!(e.kind(), std::io::ErrorKind::Unsupported) {
                return Err(LinkFailure::Unsupported(
                    "The filesystem holding the game does not support symbolic links.".into(),
                ));
            }

            Err(LinkFailure::Io(e))
        }
    }
}

/// Copy `src` to `dst`, creating the parent directory.
pub fn copy_file(src: &Path, dst: &Path) -> Result<(), LinkFailure> {
    if let Some(parent) = dst.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(LinkFailure::Io(e));
        }
    }

    std::fs::copy(src, dst).map(|_| ()).map_err(LinkFailure::Io)
}

/// Place one file by whichever mechanism `kind` names.
pub fn place(kind: LinkKind, src: &Path, dst: &Path) -> Result<(), LinkFailure> {
    if let Some(parent) = dst.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(LinkFailure::Io(e));
        }
    }

    match kind {
        LinkKind::Hard => hard_link(src, dst),
        LinkKind::Symbolic => symlink_file(src, dst),
        LinkKind::Copy => copy_file(src, dst),
    }
}

// ------------------------------------------------------------- Identity

/// Is `path` itself a symbolic link? (Not: does it point at one.)
pub fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// Do these two paths name the same file?
///
/// The only way to recognise a hard link we created, since a hard link is not
/// marked in any way — both names are equally the file.
///
/// Unix compares `(dev, ino)`, which is exact. Windows has no inode reachable
/// from `std`, so it compares length and modification time; that can produce a
/// FALSE POSITIVE for two files written in the same instant with the same
/// size, which is why the purge path treats a match as permission to remove
/// only a path it also recorded in its own ledger.
pub fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        ma.dev() == mb.dev() && ma.ino() == mb.ino()
    }

    #[cfg(not(unix))]
    {
        ma.len() == mb.len() && ma.modified().ok() == mb.modified().ok()
    }
}

/// Where a symbolic link points, or `None` if it is not one.
pub fn link_target(path: &Path) -> Option<PathBuf> {
    if !is_symlink(path) {
        return None;
    }

    std::fs::read_link(path).ok()
}

// ------------------------------------------------------------ Probing

/// What this machine can actually do between two particular directories.
///
/// Probed rather than assumed. The answer depends on the filesystem under each
/// path, on whether the two are the same volume, and on a Windows privilege —
/// none of which is knowable from the target triple, and all of which the user
/// needs told BEFORE they pick a strategy in a dropdown rather than after a
/// deploy fails halfway.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkSupport {
    pub hard: bool,
    pub symbolic: bool,
    pub same_volume: bool,
    /// Why `hard` or `symbolic` is false, when it is.
    pub notes: Vec<String>,
}

/// Try each mechanism for real, in a temporary file, and report what worked.
///
/// An actual attempt rather than a rules table: every rules table for this is
/// wrong somewhere — a Linux box with the staging folder on an exFAT USB
/// drive, a macOS volume with links disabled, a Windows machine with Developer
/// Mode on. One byte written and removed answers all of them.
pub fn probe(staging: &Path, target: &Path) -> LinkSupport {
    let mut notes = Vec::new();

    let same = match same_volume(staging, target) {
        Ok(same) => same,
        Err(e) => {
            notes.push(e.to_string());
            false
        }
    };

    let probe_dir = target.join(".tmc-probe");

    if std::fs::create_dir_all(&probe_dir).is_err() {
        notes.push("The game folder is not writable by this app.".into());

        return LinkSupport {
            hard: false,
            symbolic: false,
            same_volume: same,
            notes,
        };
    }

    let source = staging.join(".tmc-probe-src");
    let mut hard = false;
    let mut symbolic = false;

    if std::fs::create_dir_all(staging).is_ok() && std::fs::write(&source, b"tmc").is_ok() {
        let hard_at = probe_dir.join("hard");
        let sym_at = probe_dir.join("sym");

        match hard_link(&source, &hard_at) {
            Ok(()) => hard = true,
            Err(e) => notes.push(format!("Hard links: {}", e.message())),
        }

        match symlink_file(&source, &sym_at) {
            Ok(()) => symbolic = true,
            Err(e) => notes.push(format!("Symbolic links: {}", e.message())),
        }

        let _ = std::fs::remove_file(&hard_at);
        let _ = std::fs::remove_file(&sym_at);
        let _ = std::fs::remove_file(&source);
    } else {
        notes.push("The staging folder is not writable by this app.".into());
    }

    let _ = std::fs::remove_dir_all(&probe_dir);

    LinkSupport {
        hard,
        symbolic,
        same_volume: same,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_ancestor_is_found_through_missing_children() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let deep = tmp.path().join("a/b/c/d.txt");

        assert_eq!(
            existing_ancestor(&deep).as_deref(),
            Some(tmp.path()),
            "should walk up to the directory that exists"
        );
    }

    #[test]
    fn a_directory_is_on_the_same_volume_as_itself() {
        let tmp = tempfile::tempdir().expect("tempdir");

        assert!(same_volume(tmp.path(), tmp.path()).expect("same volume"));
    }

    #[test]
    fn a_hard_link_shares_identity_with_its_source() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let src = tmp.path().join("src.bin");
        let dst = tmp.path().join("dst.bin");

        std::fs::write(&src, b"payload").expect("write");

        // A filesystem with no hard links is a legitimate environment; skip
        // rather than fail, since the point of the test is the identity check.
        if hard_link(&src, &dst).is_err() {
            return;
        }

        assert!(same_file(&src, &dst));
        assert!(!is_symlink(&dst), "a hard link is not a symlink");

        let other = tmp.path().join("other.bin");
        std::fs::write(&other, b"payload").expect("write");

        #[cfg(unix)]
        assert!(
            !same_file(&src, &other),
            "two separate files must not read as the same one"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_recognised_and_readable() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let src = tmp.path().join("src.bin");
        let dst = tmp.path().join("dst.bin");

        std::fs::write(&src, b"payload").expect("write");
        symlink_file(&src, &dst).expect("symlink");

        assert!(is_symlink(&dst));
        assert_eq!(link_target(&dst).as_deref(), Some(src.as_path()));
        // Follows the link, so it is the same file.
        assert!(same_file(&src, &dst));
    }

    #[test]
    fn a_probe_reports_something_for_a_plain_temp_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let staging = tmp.path().join("staging");
        let target = tmp.path().join("game");

        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::create_dir_all(&target).expect("target");

        let support = probe(&staging, &target);

        assert!(support.same_volume, "one temp dir, one volume");

        // The probe must clean up after itself — a stray `.tmc-probe` inside
        // somebody's game folder is exactly the kind of litter that gets an
        // app blamed for a broken install.
        assert!(!target.join(".tmc-probe").exists());
        assert!(!staging.join(".tmc-probe-src").exists());
    }
}
