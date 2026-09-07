//! Everything the TMC app does that is not "being a window".
//!
//! This crate deliberately has **no Tauri dependency**. The split is not
//! cosmetic: the code that matters most here is the plugin jail, the query
//! parsers and the auth token lifecycle, and all three are exactly the code you
//! want to be able to compile and fuzz on a CI runner with no display stack,
//! no WebKit and no dbus. Keeping Tauri on the other side of the boundary is
//! what makes `cargo test -p tmc-core` work anywhere.
//!
//! The Tauri crate above it owns four things and no more: the window, the path
//! resolver, the `#[tauri::command]` surface, and the assembly of [`AppState`]
//! from the pieces here.
//!
//! [`AppState`]: ../tmc_app_lib/state/struct.AppState.html

pub mod anchor;
pub mod api;
pub mod auth;
pub mod crypto;
pub mod deeplink;
pub mod deploy;
pub mod detect;
pub mod download;
pub mod error;
pub mod launch;
pub mod library;
pub mod local;
pub mod logging;
pub mod net;
pub mod plugins;
pub mod rcon;
pub mod secure;
pub mod session;
pub mod settings;

pub use error::{AppError, AppResult};
