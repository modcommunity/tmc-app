//! Keeping the device's library in step with the account's.
//!
//! WHY THIS POLLS
//! --------------
//! A subscription can be created in a BROWSER — on the website, on a phone, by
//! clicking Subscribe on a mod page — and the device has no other way to hear
//! about it. There is no push channel to an installed desktop app that is
//! reliable across three desktop platforms, two mobile ones, corporate
//! firewalls and sleeping laptops, so the app asks. Once a minute while it is
//! open, and once on launch.
//!
//! WHAT MAKES THAT CHEAP
//! ---------------------
//! A watermark. Each response carries a `revision`; the next request sends it
//! back and the server answers with only what changed since. A device that has
//! been open for an hour has made sixty requests and transferred one row.
//!
//! WHY A FULL SYNC STILL HAPPENS
//! -----------------------------
//! A delta cannot express a DELETION — there is nothing left to poll. So every
//! `FULL_SYNC_EVERY` passes (and always on launch) the app asks for the whole
//! list and reconciles: anything it holds that the server did not send has been
//! unsubscribed elsewhere, and is uninstalled.
//!
//! WHAT THIS MODULE DOES *NOT* DO
//! ------------------------------
//! Write to disk. It reconciles the DATABASE and returns a plan; the installer
//! ([`super::install`]) is what touches the filesystem, behind the jail. The
//! split is what lets the whole of this be tested without a game directory.

use serde::{Deserialize, Serialize};

use crate::api::{ApiClient, Method};
use crate::error::AppResult;

use super::db::{InstallMirror, LibraryDb, LibraryEntry};

/// How often a poll asks for the whole list rather than a delta.
///
/// Every tenth pass — so with the default one-minute interval a subscription
/// removed on the website is uninstalled here within ten minutes, without the
/// other nine passes carrying the whole library.
pub const FULL_SYNC_EVERY: u32 = 10;

/// The meta key holding the watermark.
const REVISION_KEY: &str = "subscriptions.revision";

/// The wire shape. Mirrors `SubscriptionSchema` in the contract; only the
/// fields this side uses are named, because `serde` ignores the rest and a
/// field added server-side must not break an installed app.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSubscription {
    id: String,
    kind: String,
    item_id: i64,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    image: Option<String>,
    web_url: String,
    #[serde(default)]
    app: Option<WireApp>,
    #[serde(default)]
    created_at: String,
    updated_at: String,
    auto_update: bool,
    notify_updates: bool,
    paused: bool,
    #[serde(default)]
    via_collection_id: Option<i64>,
    installable: bool,
    #[serde(default)]
    release: Option<WireRelease>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireApp {
    id: i64,
    name: String,
    #[serde(default)]
    slug: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireRelease {
    id: i64,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    file: Option<WireFile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireFile {
    #[serde(default)]
    url: String,
    #[serde(default)]
    size: Option<i64>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSyncResponse {
    #[serde(default)]
    items: Vec<WireSubscription>,
    #[serde(default)]
    revision: String,
    #[serde(default)]
    full: bool,
}

impl WireSubscription {
    fn into_entry(self) -> LibraryEntry {
        let (release_id, version, file) = match self.release {
            Some(rel) => (Some(rel.id), rel.version, rel.file),
            None => (None, None, None),
        };

        let _ = self.created_at;

        LibraryEntry {
            id: self.id,
            kind: self.kind,
            item_id: self.item_id,
            name: self.name,
            description: self.description,
            image: self.image,
            web_url: self.web_url,
            app_id: self.app.as_ref().map(|a| a.id),
            app_name: self.app.as_ref().map(|a| a.name.clone()),
            /*
             * Lower-cased HERE as well as server-side. The slug selects a
             * directory under `plugins/app/`, and a path on Linux is
             * case-sensitive — so a server that ever stops normalising would
             * turn every install for that game into "no rule for this app"
             * rather than an error anybody could diagnose.
             */
            app_slug: self
                .app
                .and_then(|a| a.slug)
                .map(|s| s.to_ascii_lowercase()),
            auto_update: self.auto_update,
            notify_updates: self.notify_updates,
            paused: self.paused,
            via_collection_id: self.via_collection_id,
            installable: self.installable,
            latest_release_id: release_id,
            latest_version: version,
            file_url: file.as_ref().map(|f| f.url.clone()),
            file_name: file.as_ref().and_then(|f| f.name.clone()),
            file_size: file.as_ref().and_then(|f| f.size),
            file_sha256: file.and_then(|f| f.sha256),
            updated_at: self.updated_at,

            // Device-local columns are never sent by the server and are never
            // written by an upsert — see `LibraryDb::upsert_remote`.
            installed_release_id: None,
            installed_version: None,
            installed_install_id: None,
            installed_at: None,
            installed_files: vec![],
            state: "idle".into(),
            last_error: None,
        }
    }
}

/// What one sync pass changed, and what the installer should do next.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    /// Rows the server sent.
    pub received: usize,
    /// Rows that were new to this device.
    pub added: usize,
    /// Rows whose newest release moved past what is installed.
    pub updated: usize,
    /// Rows the server no longer lists (full syncs only).
    pub removed: usize,
    /// Whether this was a full list rather than a delta.
    pub full: bool,
    /// Subscription ids that now want installing or updating.
    pub to_install: Vec<String>,
    /// Subscription ids that were on disk and should be removed.
    pub to_uninstall: Vec<String>,
    /// Ids the server no longer lists at all, so their ROW goes too once their
    /// files are off disk. A subset of `to_uninstall`; a paused subscription is
    /// never in here, because the user still has it.
    pub to_forget: Vec<String>,
}

/// Run one sync pass.
///
/// `force_full` is what the launch pass and the "Sync now" button set. Every
/// other pass is a delta unless the watermark is missing, which is what a fresh
/// install and a re-login both look like.
pub async fn sync_once(api: &ApiClient, db: &LibraryDb, force_full: bool) -> AppResult<SyncReport> {
    let since = if force_full {
        None
    } else {
        db.meta_get(REVISION_KEY)?
    };

    let path = match &since {
        Some(rev) if !rev.is_empty() => {
            format!("/subscriptions?limit=200&since={}", urlencode(rev))
        }
        _ => "/subscriptions?limit=200".to_string(),
    };

    let value = api.request(Method::GET, &path, None, true).await?;

    let response: WireSyncResponse = serde_json::from_value(value)?;

    let mut report = SyncReport {
        received: response.items.len(),
        full: response.full,
        ..Default::default()
    };

    let mut seen: Vec<String> = Vec::with_capacity(response.items.len());

    for wire in response.items {
        let entry = wire.into_entry();

        seen.push(entry.id.clone());

        let before = db.get(&entry.id)?;

        db.upsert_remote(&entry)?;

        // Re-read: the upsert deliberately preserves the device-local half, so
        // the merged row is the only one that can answer "does this need
        // installing?".
        let Some(after) = db.get(&entry.id)? else {
            continue;
        };

        if before.is_none() {
            report.added += 1;
        } else if after.update_available() {
            report.updated += 1;
        }

        /*
         * The install plan. Three conditions, all of them the user's own
         * settings rather than ours:
         *
         *   * `should_install` — installable, not paused, and there is a file.
         *   * either nothing is on disk, or a newer release exists.
         *   * `auto_update` — off means "tell me, do not touch it", so an
         *     update is reported by the UI and not queued here.
         */
        if after.should_install()
            && (after.installed_release_id.is_none()
                || (after.update_available() && after.auto_update))
        {
            report.to_install.push(after.id.clone());
        }

        /*
         * A subscription that is still listed but has become un-installable, or
         * has been paused, is REMOVED from disk rather than left. That is the
         * point of pausing — "take it out of my game, keep it in my list" — and
         * an item whose team disabled subscriptions should stop being applied.
         */
        if after.installed_release_id.is_some() && (after.paused || !after.installable) {
            report.to_uninstall.push(after.id.clone());
        }
    }

    if response.full {
        for gone in db.missing_from(&seen)? {
            report.removed += 1;

            if gone.installed_release_id.is_some() {
                // Uninstall FIRST, delete the row second — the file list lives
                // on the row, and dropping it here would strand the files with
                // nothing left that knows how to remove them.
                report.to_uninstall.push(gone.id.clone());
                report.to_forget.push(gone.id.clone());
            } else {
                db.delete(&gone.id)?;
            }
        }
    }

    if !response.revision.is_empty() {
        db.meta_set(REVISION_KEY, &response.revision)?;
    }

    Ok(report)
}

/// Pull the account's installs down into the local mirror.
///
/// The cloud row is the definition; the mirror exists so the app can render and
/// launch an install while offline, and so `local_dir` — which is this
/// machine's answer and nobody else's — has somewhere to live.
pub async fn sync_installs(api: &ApiClient, db: &LibraryDb) -> AppResult<usize> {
    let value = api.request(Method::GET, "/installs", None, true).await?;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Wire {
        #[serde(default)]
        installs: Vec<serde_json::Value>,
    }

    let wire: Wire = serde_json::from_value(value)?;

    let mut keep: Vec<i64> = Vec::with_capacity(wire.installs.len());

    for install in &wire.installs {
        let Some(id) = install.get("id").and_then(serde_json::Value::as_i64) else {
            continue;
        };

        let app_id = install
            .get("appId")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default();

        let app_slug = install
            .get("app")
            .and_then(|a| a.get("slug"))
            .and_then(serde_json::Value::as_str);

        let name = install
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Install");

        let is_default = install
            .get("isDefault")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let game_version = install
            .get("gameVersion")
            .and_then(serde_json::Value::as_str);
        let loader = install.get("loader").and_then(serde_json::Value::as_str);

        let updated_at = install
            .get("updatedAt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();

        db.upsert_install(&InstallMirror {
            id,
            app_id,
            app_slug,
            name,
            is_default,
            game_version,
            loader,
            updated_at,
            payload: &install.to_string(),
        })?;

        /*
         * And the sandbox the mod manager works from.
         *
         * The raw payload above is kept as well as this, not instead of it: the
         * payload is what the UI parses with the contract's own zod schema, and
         * re-deriving it from these columns would be a second shape that can
         * drift. This is the structured half the deployment engine needs.
         */
        if let Err(e) = mirror_sandbox(db, install, id, app_id, app_slug, name, is_default) {
            tracing::warn!("could not mirror install {id} as a sandbox: {}", e.detail());
        }

        keep.push(id);
    }

    db.retain_installs(&keep)?;
    db.sandbox_retain_remote(&keep)?;

    Ok(keep.len())
}

/// Turn one cloud install payload into a sandbox row.
#[allow(clippy::too_many_arguments)]
fn mirror_sandbox(
    db: &LibraryDb,
    payload: &serde_json::Value,
    remote_id: i64,
    app_id: i64,
    app_slug: Option<&str>,
    name: &str,
    is_default: bool,
) -> AppResult<()> {
    use crate::deploy::Strategy;
    use crate::library::sandbox::{Environment, RemoteItem, RemoteSandbox};

    let text = |key: &str| payload.get(key).and_then(serde_json::Value::as_str);

    let json = |key: &str| {
        payload
            .get(key)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "{}".into())
    };

    /*
     * `environment` and `strategy` are read with a fallback rather than
     * required. An installed app talks to whatever version of the API is
     * deployed, and a server that predates these fields must produce a working
     * sandbox rather than none — the defaults are the same ones a locally
     * created sandbox gets.
     */
    let environment = text("environment")
        .and_then(Environment::parse)
        .unwrap_or_default();

    let strategy = text("strategy")
        .and_then(Strategy::parse)
        .unwrap_or_default();

    let items: Vec<RemoteItem> = payload
        .get("items")
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|item| {
                    Some(RemoteItem {
                        kind: item.get("kind")?.as_str()?.to_string(),
                        item_id: item.get("itemId")?.as_i64()?,
                        name: item
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("Item")
                            .to_string(),
                        enabled: item
                            .get("enabled")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(true),
                        order: item
                            .get("order")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let app_name = payload
        .get("app")
        .and_then(|a| a.get("name"))
        .and_then(serde_json::Value::as_str);

    db.sandbox_upsert_remote(&RemoteSandbox {
        remote_id,
        app_id,
        app_slug,
        app_name,
        name,
        description: text("description"),
        environment,
        strategy,
        game_version: text("gameVersion"),
        loader: text("loader"),
        is_default,
        options: json("options"),
        launch_args: payload
            .get("launchArgs")
            .map(|v| v.to_string())
            .unwrap_or_else(|| "[]".into()),
        launch_env: json("launchEnv"),
        items: &items,
    })?;

    Ok(())
}

/// Percent-encode a watermark for a query string.
///
/// Hand-rolled rather than pulling `percent-encoding` in for one call: the
/// value is a decimal timestamp we produced, so this is belt-and-braces against
/// the format changing later, not a general encoder.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());

    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(id: &str, release: Option<i64>) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "kind": "mod",
            "itemId": 42,
            "name": "Cool Mod",
            "webUrl": "https://example.com/m/42",
            "app": { "id": 1, "name": "Minecraft", "slug": "MineCraft" },
            "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-02T00:00:00Z",
            "autoUpdate": true,
            "notifyUpdates": true,
            "paused": false,
            "installable": true,
            "release": release.map(|id| serde_json::json!({
                "id": id,
                "version": "1.4",
                "file": {
                    "url": "https://example.com/download/abc",
                    "size": 1024,
                    "sha256": null,
                    "name": "cool.jar"
                }
            })),
        })
    }

    #[test]
    fn a_slug_is_normalised_on_the_way_in() {
        let parsed: WireSubscription = serde_json::from_value(wire("s1", Some(7))).expect("parse");

        assert_eq!(parsed.into_entry().app_slug.as_deref(), Some("minecraft"));
    }

    #[test]
    fn a_row_with_no_release_is_not_installable_yet() {
        let parsed: WireSubscription = serde_json::from_value(wire("s1", None)).expect("parse");

        let entry = parsed.into_entry();

        assert_eq!(entry.latest_release_id, None);
        assert!(!entry.should_install());
    }

    #[test]
    fn unknown_server_fields_do_not_break_an_installed_app() {
        let mut value = wire("s1", Some(7));

        value["somethingNew"] = serde_json::json!({ "added": "later" });

        let parsed: Result<WireSubscription, _> = serde_json::from_value(value);

        assert!(parsed.is_ok());
    }

    #[test]
    fn urlencode_leaves_a_plain_watermark_alone_and_escapes_the_rest() {
        assert_eq!(urlencode("1767225600000"), "1767225600000");
        assert_eq!(urlencode("a b&c"), "a%20b%26c");
    }
}
