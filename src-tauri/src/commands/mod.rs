//! The complete IPC surface, split by subject.
//!
//! Two rules hold across all of it:
//!
//!   * **No command returns a credential.** There is no `get_token`. The
//!     frontend asks for data and gets data; the bearer is attached inside
//!     `tmc_core::api`. That is what makes an injected script in a rendered mod
//!     description a nuisance rather than an account compromise.
//!   * **No command takes a filesystem path from the webview**, except the
//!     folder the user picked and [`fs`]'s directory listing, and neither is
//!     ever written through.
//!
//! [`fs`] is the one deliberate widening of the second rule and documents its
//! own cost at the top of the module. Read that before adding anything beside
//! it.
//!
//! `lib.rs` names each command by its full module path rather than through a
//! re-export here: `generate_handler!` also needs the hidden `__cmd__*` items
//! the `#[tauri::command]` macro generates alongside the function, and those do
//! not travel through a `pub use`.

pub mod api;
pub mod auth;
pub mod config;
pub mod detect;
pub mod downloads;
pub mod fs;
pub mod import;
pub mod library;
pub mod logs;
pub mod play;
pub mod plugins;
pub mod rcon;
pub mod sandbox;
pub mod servers;
pub mod sessions;
pub mod settings;
