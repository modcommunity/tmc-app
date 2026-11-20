//! The avatar studio's commands (docs/avatar.md).
//!
//! Each read names a fixed `/api/avatar/v1` route; the webview passes a game's
//! numeric id at most. `avatar_asset` is the one command here that takes a
//! URL, and only to say WHICH of the files TMC already named it wants: the
//! allowlist is rebuilt in Rust from TMC's own answers, never from anything
//! the webview sent, and the bytes come back over IPC so the webview's CSP
//! keeps `connect-src` to itself.

use std::collections::HashSet;
use std::sync::OnceLock;

use serde_json::Value;
use tauri::State;

use tmc_core::api::{api_base, Method};
use tmc_core::avatar::{allowlist, is_allowed, AllowCache, AvatarRoute};
use tmc_core::error::{AppError, AppResult};

use crate::state::AppState;

fn cache() -> &'static AllowCache {
    static CACHE: OnceLock<AllowCache> = OnceLock::new();

    CACHE.get_or_init(AllowCache::default)
}

/// TMC's built-in figure (`GET /api/avatar/v1/manifest`); null when TMC has none.
#[tauri::command]
pub async fn avatar_manifest(state: State<'_, AppState>) -> AppResult<Option<Value>> {
    state
        .api
        .avatar(Method::GET, AvatarRoute::Manifest, None, None, false)
        .await
}

/// One game's avatar library; null when avatars are off for it.
#[tauri::command]
pub async fn avatar_library(state: State<'_, AppState>, app_id: u64) -> AppResult<Option<Value>> {
    state
        .api
        .avatar(Method::GET, AvatarRoute::Library, Some(app_id), None, false)
        .await
}

/// The member's avatar for one game; null when none is saved.
#[tauri::command]
pub async fn avatar_me(state: State<'_, AppState>, app_id: u64) -> AppResult<Option<Value>> {
    state
        .api
        .avatar(Method::GET, AvatarRoute::Me, Some(app_id), None, true)
        .await
}

/// Saves the member's avatar for one game. TMC validates the body.
#[tauri::command]
pub async fn avatar_me_put(state: State<'_, AppState>, body: Value) -> AppResult<Value> {
    Ok(state
        .api
        .avatar(Method::PUT, AvatarRoute::Me, None, Some(body), true)
        .await?
        .unwrap_or(Value::Null))
}

/// One model file's bytes, if TMC named it: in the manifest, in game
/// `app_id`'s library, or (with `me`) among the member's own wearables.
/// Anything else is refused before a request leaves.
#[tauri::command]
pub async fn avatar_asset(
    state: State<'_, AppState>,
    url: String,
    app_id: Option<u64>,
    me: Option<bool>,
) -> AppResult<tauri::ipc::Response> {
    let base = api_base();
    let key = app_id.unwrap_or(0);

    let mut allowed = match cache().get(key) {
        Some(set) => set,
        None => {
            let manifest = public(&state, AvatarRoute::Manifest, None).await;
            let library = match app_id {
                Some(id) => public(&state, AvatarRoute::Library, Some(id)).await,
                None => None,
            };
            let set = allowlist(manifest.as_ref(), library.as_ref(), None, base);

            cache().put(key, set.clone());
            set
        }
    };

    if me.unwrap_or(false) && !is_allowed(&allowed, &url, base) {
        // Not cached: equipping or buying a wearable changes this list.
        let own = state
            .api
            .request(Method::GET, "/avatar/wearables", None, true)
            .await
            .ok();
        let mine: HashSet<String> = allowlist(None, None, own.as_ref(), base);

        allowed.extend(mine);
    }

    if !is_allowed(&allowed, &url, base) {
        return Err(AppError::invalid("Not an avatar file TMC named."));
    }

    let bytes = state.api.avatar_file(&url).await?;

    Ok(tauri::ipc::Response::new(bytes))
}

/// A public avatar document for the allowlist; None when TMC failed, which
/// only narrows the list.
async fn public(state: &AppState, route: AvatarRoute, app_id: Option<u64>) -> Option<Value> {
    state
        .api
        .avatar(Method::GET, route, app_id, None, false)
        .await
        .ok()
        .flatten()
}
