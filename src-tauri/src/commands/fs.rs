//! Directory listing, for the app's own folder picker.
//!
//! WHAT THIS DELIBERATELY WIDENS
//! -----------------------------
//! The rule at the top of `commands/mod.rs` is that no command takes a
//! filesystem path from the webview. This module is the exception, and it is
//! worth stating plainly what it costs: a script running in the webview can
//! walk the directory TREE of the machine, one level per call.
//!
//! What it still cannot do, and what keeps the exception affordable:
//!
//!   * **No file is ever listed.** Only directories come back — not names,
//!     sizes, timestamps or extensions of files. "Is there a `taxes-2024.pdf`
//!     in Documents?" is not answerable through this.
//!   * **Nothing is read, written, created or removed.** The command has no
//!     path into the plugin executor, the download cache or the settings store.
//!   * **Hidden directories are omitted**, which keeps `~/.ssh`, `~/.config`
//!     and the rest of a dotfile tree out of the reply entirely.
//!   * **Every listing is capped** at [`MAX_ENTRIES`], so a directory with a
//!     hundred thousand children cannot be used to stall the app or balloon the
//!     IPC reply.
//!   * **It makes nothing writable.** Reading the tree is the whole capability.
//!     The jail anchors this picker feeds — `gameDirs` and `downloadDir` — are
//!     refused by `settings_patch` (`JAIL_ROOT_FIELDS`) and reachable only
//!     through their own setters, each of which runs
//!     [`tmc_core::anchor::validate_root`] on the result. So a directory this
//!     lists is a directory the webview may NAME; whether it may become a jail
//!     anchor is a separate decision made somewhere else, against rules this
//!     module has no part in.
//!
//! WHY IT EXISTS
//! -------------
//! The alternative is `tauri-plugin-dialog`'s native picker, which on Linux is
//! a GTK file chooser: themed by the user's desktop, matching neither the app
//! nor the other four platforms, and the single largest piece of GTK left on
//! screen once the window frame is drawn by the app itself.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use tmc_core::audit;
use tmc_core::error::{AppError, AppResult};

use crate::state::AppState;

/// Directories returned in one listing.
///
/// A picker shows a scrolling list; nobody scrolls past two thousand folders,
/// and the cap is what stops `/nix/store` or a node_modules tree turning one
/// click into a multi-megabyte IPC reply.
const MAX_ENTRIES: usize = 2_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntryInfo {
    /// The last component, for display.
    pub name: String,
    /// The absolute path, which the caller hands straight back to descend.
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    pub path: String,
    /// The last component of `path`, or the path itself at a filesystem root.
    pub name: String,
    /// `None` at a root, which is what disables the picker's "up" button.
    pub parent: Option<String>,
    pub entries: Vec<DirEntryInfo>,
    /// True when [`MAX_ENTRIES`] cut the listing short. The picker says so
    /// rather than silently showing a prefix — a folder the user knows is
    /// there and cannot find reads as a bug in the picker.
    pub truncated: bool,
    /// The listing was refused by the OS. Present instead of an error so the
    /// picker can show the crumb and let the user go back up, rather than
    /// dead-ending on a permissions dialog.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied: Option<String>,
}

/// A place the picker offers to start from.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirRoot {
    pub label: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListRequest {
    /// Absolute path to list. `None` starts at the user's home directory.
    #[serde(default)]
    pub path: Option<String>,
    /// Show directories whose name starts with a dot. Off by default; the
    /// picker offers it as a toggle for the games that install into one.
    #[serde(default)]
    pub show_hidden: bool,
}

/// The places worth offering as a starting point.
///
/// Home first, then the filesystem root — and on Windows every drive letter
/// that answers, since "the game is on D:" is the common case there and there
/// is no single root to walk down from.
#[tauri::command]
pub fn fs_roots(app: AppHandle) -> Vec<DirRoot> {
    let mut roots: Vec<DirRoot> = Vec::new();

    if let Ok(home) = app.path().home_dir() {
        roots.push(DirRoot {
            label: "Home".into(),
            path: home.to_string_lossy().into_owned(),
        });
    }

    if let Ok(downloads) = app.path().download_dir() {
        roots.push(DirRoot {
            label: "Downloads".into(),
            path: downloads.to_string_lossy().into_owned(),
        });
    }

    #[cfg(windows)]
    {
        for letter in b'A'..=b'Z' {
            let path = format!("{}:\\", letter as char);

            if Path::new(&path).is_dir() {
                roots.push(DirRoot {
                    label: format!("{}:", letter as char),
                    path,
                });
            }
        }
    }

    #[cfg(not(windows))]
    {
        roots.push(DirRoot {
            label: "Filesystem".into(),
            path: "/".into(),
        });
    }

    roots
}

/// List the directories directly inside `path`.
///
/// Directories only, by construction rather than by filter: the entry loop
/// never asks a non-directory for anything, so there is no branch in which a
/// filename could reach the reply.
#[tauri::command]
pub fn fs_list_dirs(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ListRequest,
) -> AppResult<DirListing> {
    let start = match request
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(path) => PathBuf::from(path),
        None => app
            .path()
            .home_dir()
            .map_err(|_| AppError::invalid("This machine has no home directory."))?,
    };

    /*
     * Canonicalised before anything else, so the path that goes into the reply
     * — and therefore the one that comes back on the next call and eventually
     * lands in `gameDirs` — is the real one rather than a chain of `..` and
     * symlinks. The plugin jail canonicalises its own roots too; agreeing
     * with it here means the picker cannot show a path the jail would then
     * resolve somewhere else.
     */
    let dir = start
        .canonicalize()
        .map_err(|_| AppError::invalid("That folder does not exist."))?;

    if !dir.is_dir() {
        return Err(AppError::invalid("That is not a folder."));
    }

    let mut entries: Vec<DirEntryInfo> = Vec::new();
    let mut truncated = false;
    let mut denied = None;

    match std::fs::read_dir(&dir) {
        Ok(reader) => {
            for entry in reader {
                // A single unreadable entry is skipped rather than failing the
                // whole listing: one root-owned directory in `/` must not make
                // `/` unbrowsable.
                let Ok(entry) = entry else { continue };

                /*
                 * `file_type` does NOT follow symlinks, and that is deliberate.
                 * `is_dir` on the metadata would, which turns a symlink loop
                 * into an unbounded walk and lets a link in a shared directory
                 * present someone else's tree as a subfolder of the user's own.
                 * A link to a directory is simply not offered.
                 */
                let Ok(kind) = entry.file_type() else {
                    continue;
                };

                if !kind.is_dir() {
                    continue;
                }

                let name = entry.file_name().to_string_lossy().into_owned();

                if !request.show_hidden && name.starts_with('.') {
                    continue;
                }

                if entries.len() >= MAX_ENTRIES {
                    truncated = true;
                    break;
                }

                entries.push(DirEntryInfo {
                    path: entry.path().to_string_lossy().into_owned(),
                    name,
                });
            }
        }
        Err(err) => denied = Some(err.to_string()),
    }

    // Case-insensitive, so `Steam` and `games` sort the way a person expects
    // rather than the way ASCII does.
    entries.sort_by_key(|e| e.name.to_lowercase());

    audit!(
        state.audit,
        Debug,
        App,
        "fs.list",
        format!("Listed {} folders in {}", entries.len(), dir.display())
    );

    Ok(DirListing {
        name: display_name(&dir),
        parent: dir
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|p| !p.is_empty()),
        path: dir.to_string_lossy().into_owned(),
        entries,
        truncated,
        denied,
    })
}

/// The crumb label for a directory: its last component, or the whole path at a
/// filesystem root, where there is no last component to show.
fn display_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_root_directory_reports_no_parent_and_names_itself() {
        let root = Path::new("/");

        assert_eq!(display_name(root), "/");
        assert!(root.parent().is_none());
    }

    #[test]
    fn an_ordinary_directory_is_named_by_its_last_component() {
        assert_eq!(display_name(Path::new("/home/someone/Games")), "Games");
    }
}
