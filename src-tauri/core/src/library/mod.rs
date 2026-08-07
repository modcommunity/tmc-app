//! The device's **library**: which subscriptions this machine holds, which of
//! them are on disk, and how they got there.
//!
//! Three pieces, deliberately separable:
//!
//! | Module | Owns | Touches the disk? |
//! | --- | --- | --- |
//! | [`db`] | The SQLite store | Only its own database file |
//! | [`sync`] | Reconciling against the account | No |
//! | [`install`] | Turning a subscription into files | Through the sandbox only |
//!
//! The split is what lets the interesting half be tested with no game
//! directory, no network and no display stack: `sync` produces a PLAN and
//! `install` executes it, and only the second one needs a filesystem.

pub mod db;
pub mod install;
pub mod sync;

pub use db::{LibraryDb, LibraryEntry};
pub use install::{InstallCtx, InstallOutcome};
pub use sync::{sync_installs, sync_once, SyncReport, FULL_SYNC_EVERY};
