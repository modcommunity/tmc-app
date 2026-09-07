//! Handing the virtual tree to a process that is not ours.
//!
//! The app builds the tree; the GAME has to read it, from inside its own
//! address space, in a hook that runs before most of the game's own
//! initialisation. That is the whole problem this module solves.
//!
//! WHY A FILE AND NOT A PIPE OR A SOCKET
//! ------------------------------------
//! The hook runs inside `CreateFileW`. Anything it does that could itself open
//! a file, take a lock, or wait on another process is a re-entrancy hazard or a
//! deadlock in a code path the game calls thousands of times a second. Reading
//! a memory-mapped blob once, at DLL attach, and then answering every call from
//! a plain in-process table is the only shape that is safe.
//!
//! The blob is published as a real file in the app's own data directory and
//! mapped by the injected process. On Windows it could equally be an anonymous
//! section with a named handle; a file is used because it survives inspection
//! (a support thread can be asked for it), it needs no name-squatting defence,
//! and the injector passes its path explicitly rather than the DLL guessing a
//! global name — which is the part that would otherwise be an attack surface,
//! since any process can create an object in the global namespace.
//!
//! THE FORMAT
//! ----------
//! ```text
//!   magic     8 bytes   "TMCVFS\0\1"
//!   revision  8 bytes   little-endian; changes when the tree changes
//!   length    8 bytes   little-endian; bytes of JSON that follow
//!   json      length    the serialised VirtualTree
//! ```
//!
//! JSON rather than a packed binary layout, and that is a deliberate trade. The
//! blob is read ONCE per process launch and never in a hot path, so parsing
//! cost is irrelevant; what matters is that a format mismatch between the app
//! and an injected DLL from a different build is a parse error rather than a
//! wild pointer inside somebody's game. The magic carries a version byte so
//! that mismatch is caught before the parse even runs.

use std::io::{Read, Write};
use std::path::Path;

use crate::tree::VirtualTree;

/// `TMCVFS` plus a format version. Bump the last byte on any layout change.
pub const MAGIC: [u8; 8] = *b"TMCVFS\0\x01";

/// Cap on a published blob. A quarter of a million mappings is around 40 MB of
/// JSON; this is generous and still refuses a corrupt length outright.
pub const MAX_BLOB: u64 = 256 * 1024 * 1024;

const HEADER_LEN: usize = 24;

/// A published tree, as it sits on disk.
#[derive(Debug, Clone)]
pub struct Published {
    pub revision: u64,
    pub tree: VirtualTree,
}

/// Write the tree where an injected process can map it.
///
/// Written to a temporary name and renamed, because the game may map this file
/// at any moment and a half-written blob mapped into a process is not an error
/// it can report — it is a crash with our name on it.
pub fn publish(path: &Path, revision: u64, tree: &VirtualTree) -> std::io::Result<()> {
    let json = serde_json::to_vec(tree).map_err(std::io::Error::other)?;

    if json.len() as u64 > MAX_BLOB {
        return Err(std::io::Error::other(
            "virtual tree is too large to publish",
        ));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("tmp");

    {
        let mut file = std::fs::File::create(&tmp)?;

        file.write_all(&MAGIC)?;
        file.write_all(&revision.to_le_bytes())?;
        file.write_all(&(json.len() as u64).to_le_bytes())?;
        file.write_all(&json)?;
        file.flush()?;
    }

    std::fs::rename(&tmp, path)?;

    Ok(())
}

/// Read a published tree.
///
/// Every failure is a plain `Err`. This runs inside an injected DLL in somebody
/// else's process, where a panic is not a stack trace — it is the game closing
/// with no explanation, blamed on the mod manager.
pub fn read(path: &Path) -> std::io::Result<Published> {
    let mut file = std::fs::File::open(path)?;

    let mut header = [0u8; HEADER_LEN];

    file.read_exact(&mut header)?;

    if header[..8] != MAGIC {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a TMC virtual filesystem blob, or a version this build does not read",
        ));
    }

    let revision = u64::from_le_bytes(
        header[8..16]
            .try_into()
            .map_err(|_| std::io::Error::other("short header"))?,
    );

    let length = u64::from_le_bytes(
        header[16..24]
            .try_into()
            .map_err(|_| std::io::Error::other("short header"))?,
    );

    if length > MAX_BLOB {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "declared length is implausible",
        ));
    }

    /*
     * The declared length is checked against the FILE's length before
     * allocating for it. A corrupt header claiming 200 MB would otherwise
     * reserve 200 MB inside the game's address space before failing to fill it.
     */
    let available = file.metadata()?.len().saturating_sub(HEADER_LEN as u64);

    if length > available {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "blob is shorter than its header claims",
        ));
    }

    let mut json = vec![0u8; length as usize];

    file.read_exact(&mut json)?;

    let tree: VirtualTree = serde_json::from_slice(&json).map_err(std::io::Error::other)?;

    Ok(Published { revision, tree })
}

/// The environment variable the injector uses to tell the DLL where to look.
///
/// An explicit hand-off rather than a well-known path or a global object name.
/// Any process can create an object in Windows' global namespace, so a DLL that
/// guessed a name could be handed a tree by something that is not us — and a
/// tree is a list of "when the game opens A, give it B", which is as good as
/// arbitrary file substitution inside the game.
///
/// The variable is set on the CHILD only, by the injector, and is not inherited
/// by anything the game did not itself launch from that environment.
pub const ENV_BLOB: &str = "TMC_VFS_BLOB";

/// The same, for the game directory — so the DLL can bail out immediately if it
/// finds itself in a process that is not the one it was meant for.
pub const ENV_ROOT: &str = "TMC_VFS_ROOT";

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> VirtualTree {
        let mut tree = VirtualTree::new("C:/Games/X");

        tree.insert("Data/a.esp", "D:/staging/a/Data/a.esp");
        tree.insert("Data/b.esp", "D:/staging/b/Data/b.esp");

        tree
    }

    #[test]
    fn a_published_tree_reads_back_identically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("vfs.bin");

        publish(&path, 42, &tree()).expect("publish");

        let read = read(&path).expect("read");

        assert_eq!(read.revision, 42);
        assert_eq!(read.tree.len(), 2);
        assert_eq!(
            read.tree.resolve("Data/a.esp"),
            Some("D:/staging/a/Data/a.esp")
        );
    }

    /// The game may map this file at any moment, and a half-written blob mapped
    /// into a process is not an error it can report.
    #[test]
    fn publishing_is_atomic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("vfs.bin");

        publish(&path, 1, &tree()).expect("first");

        let mut bigger = tree();

        for n in 0..500 {
            bigger.insert(&format!("Data/f{n}.esp"), &format!("D:/s/f{n}.esp"));
        }

        publish(&path, 2, &bigger).expect("second");

        let read = read(&path).expect("read");

        assert_eq!(read.revision, 2);
        assert_eq!(read.tree.len(), 502);
        // The temporary name is gone.
        assert!(!path.with_extension("tmp").exists());
    }

    /// This parses inside somebody else's game process. Every malformed input
    /// has to be an `Err`, never a panic and never a wild allocation.
    #[test]
    fn a_corrupt_blob_is_an_error_rather_than_anything_else() {
        let dir = tempfile::tempdir().expect("tempdir");

        let cases: [(&str, Vec<u8>); 6] = [
            ("empty", vec![]),
            ("short header", vec![0u8; 10]),
            ("wrong magic", {
                let mut v = vec![0u8; HEADER_LEN];
                v[..8].copy_from_slice(b"NOTVFS\0\x01");
                v
            }),
            ("absurd length", {
                let mut v = MAGIC.to_vec();
                v.extend_from_slice(&1u64.to_le_bytes());
                v.extend_from_slice(&u64::MAX.to_le_bytes());
                v
            }),
            ("length past the end", {
                let mut v = MAGIC.to_vec();
                v.extend_from_slice(&1u64.to_le_bytes());
                v.extend_from_slice(&1000u64.to_le_bytes());
                v.extend_from_slice(b"{}");
                v
            }),
            ("not json", {
                let body = b"not json at all";
                let mut v = MAGIC.to_vec();
                v.extend_from_slice(&1u64.to_le_bytes());
                v.extend_from_slice(&(body.len() as u64).to_le_bytes());
                v.extend_from_slice(body);
                v
            }),
        ];

        for (name, bytes) in cases {
            let path = dir.path().join(format!("{}.bin", name.replace(' ', "-")));

            std::fs::write(&path, &bytes).expect("write");

            assert!(read(&path).is_err(), "{name} should not read");
        }
    }

    /// A version bump has to be caught before the parse runs, not by the parse
    /// failing to make sense of a different shape.
    #[test]
    fn a_future_format_version_is_refused_by_the_magic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("vfs.bin");

        publish(&path, 1, &tree()).expect("publish");

        let mut bytes = std::fs::read(&path).expect("read");

        // Bump the version byte only.
        bytes[7] = 0x02;

        std::fs::write(&path, &bytes).expect("write");

        let err = read(&path).expect_err("must refuse");

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
