//! The complete IPC surface, split by subject.
//!
//! Two rules hold across all of it:
//!
//!   * **No command returns a credential.** There is no `get_token`. The
//!     frontend asks for data and gets data; the bearer is attached inside
//!     `tmc_core::api`. That is what makes an injected script in a rendered mod
//!     description a nuisance rather than an account compromise.
//!   * **No command takes a filesystem path from the webview**, except the one
//!     the user picked in a native folder dialog, and that one is only ever
//!     read from.
//!
//! `lib.rs` names each command by its full module path rather than through a
//! re-export here: `generate_handler!` also needs the hidden `__cmd__*` items
//! the `#[tauri::command]` macro generates alongside the function, and those do
//! not travel through a `pub use`.

pub mod api;
pub mod auth;
pub mod library;
pub mod logs;
pub mod plugins;
pub mod servers;
pub mod settings;
