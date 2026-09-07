//! **Deployment** — getting a sandbox's mods in front of the game.
//!
//! This is the half of a mod manager that everybody has and nobody agrees on.
//! MO2 hooks the filesystem, Vortex hard-links, r2modman copies per profile,
//! CurseForge points the game at a whole separate directory. Each is right for
//! the games it was built for and wrong for some of the others, so this app
//! makes it a **setting** and implements the mechanisms behind one interface.
//!
//! ```text
//!   sandbox (a profile)
//!     │  its enabled mods, in priority order
//!     ▼
//!   merge     ── the virtual tree: who wins each path, and what conflicts
//!     ▼
//!   engine    ── strategy selection, the diff against last time, placement
//!     ▼
//!   link      ── one syscall, and the platform reason it might fail
//!     │
//!   ledger    ── what landed, and what it displaced, so it can be undone
//! ```
//!
//! Three properties hold across every strategy, and they are what make the
//! whole thing safe to point at somebody's Steam folder:
//!
//!   * **Nothing is overwritten.** A file already in the game folder that the
//!     app did not put there is MOVED to the backup store before its place is
//!     taken, and a purge puts it back.
//!   * **Nothing is removed unless it is still ours.** Every removal re-checks
//!     the file against the ledger row that claims it, so a config the user
//!     edited after deploying survives.
//!   * **Staging is never modified.** Deployment only ever reads from it. That
//!     is what lets two sandboxes share one downloaded copy of a mod.
//!
//! WHERE THE FILES COME FROM
//! -------------------------
//! Not from here. An install rule (`plugins/app/<slug>/manage_mod.json`) runs
//! through the ordinary plugin executor and writes into the sandbox's **staging
//! folder** for that mod, laid out exactly as it should appear under the game
//! directory. Deployment is the second, separate step that mirrors that layout
//! into the game.
//!
//! Splitting it that way is why every install rule written before this module
//! existed still works unchanged: a rule that copies to `mods/{fileName}` puts
//! the file at `<staging>/<mod>/mods/foo.jar`, and deployment puts
//! `mods/foo.jar` in the game folder. The rule never learns which strategy the
//! sandbox uses, and it should not — "where does a Minecraft mod go" and "how
//! do files reach the game folder" are different questions with different
//! right answers.

pub mod engine;
pub mod ledger;
pub mod link;
pub mod merge;

pub use engine::{
    available_strategies, backup_root, deploy, purge, purge_vfs, resolve_strategy, stage_dir,
    stage_root, usvfs_unavailable, verify, vfs_blob, DeployReport, DeployRequest, Strategy,
    StrategyReport, VerifyReport,
};
pub use ledger::{LedgerEntry, PurgeReport};
pub use link::{LinkKind, LinkSupport};
pub use merge::{Conflict, DeployMod, MergeTree};
