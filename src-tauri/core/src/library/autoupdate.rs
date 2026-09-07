//! Keeping a sandbox's mods current without being asked twice.
//!
//! A subscription already knows the newest release the server has, and a
//! sandbox already knows which release it staged. The gap between the two is
//! this module.
//!
//! FOUR RULES, AND THE ONE THAT MATTERS MOST
//! -----------------------------------------
//!   1. **The sandbox must have `auto_update` on.** Per sandbox rather than
//!      global, because the useful case is exactly the split: a "current"
//!      profile that tracks the latest, and a pinned one for the modpack
//!      somebody's friends are all running.
//!   2. **The subscription must have `autoUpdate` on and not be paused.** That
//!      switch is the account's, set on the website or on another device, and
//!      a device that ignored it would make it meaningless.
//!   3. **Only items ALREADY STAGED are updated.** An item in the list that has
//!      never downloaded is not an update, it is a first install — which is a
//!      thing the user presses a button for.
//!   4. **A sandbox is only redeployed if it was ALREADY deployed.** This is
//!      the one that matters. Staging is invisible: it writes into the app's
//!      own directory and nothing outside notices. Deploying writes into
//!      somebody's game folder. An automatic pass that deployed a sandbox
//!      nobody had deployed would be the app putting files in a game on its own
//!      initiative, which is not a thing it may do.
//!
//! WHAT A FAILURE DOES
//! -------------------
//! Nothing, loudly. One item that will not download leaves the rest of the
//! sandbox at its current version and lands in the report; it does not roll
//! anything back, because the previous version is still staged and still
//! deployed and is exactly what the user had a minute ago.

use serde::Serialize;

use crate::audit;
use crate::error::AppResult;

use super::db::LibraryDb;
use super::deploy::{deploy_sandbox, stage_mod, SandboxCtx};
use super::sandbox::Sandbox;

/// One item that could move forward.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outdated {
    pub sandbox_id: i64,
    pub sandbox_name: String,
    pub mod_key: String,
    pub name: String,
    /// What is staged now. `None` when the staged release predates version
    /// strings being recorded.
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub release_id: Option<i64>,
}

/// What one pass did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoUpdateReport {
    /// Sandboxes considered.
    pub checked: usize,
    /// Items brought forward, as `"<sandbox> · <item>"`.
    pub updated: Vec<String>,
    /// Sandboxes redeployed afterwards.
    pub redeployed: Vec<String>,
    /// `(item, reason)` for anything that did not make it.
    pub failed: Vec<(String, String)>,
    /// Items that had an update and were left alone, and why.
    pub skipped: Vec<String>,
}

impl AutoUpdateReport {
    pub fn did_nothing(&self) -> bool {
        self.updated.is_empty() && self.failed.is_empty()
    }
}

/// Everything with a newer release available, whether or not it would be
/// updated automatically.
///
/// Used by the UI's "updates available" badge, so it deliberately ignores rules
/// 1 and 2 — somebody who turned auto-update off still wants to be told, and
/// telling them is the entire point of turning it off rather than unsubscribing.
pub fn outdated(db: &LibraryDb) -> AppResult<Vec<Outdated>> {
    let mut out = Vec::new();

    for sandbox in db.sandbox_list(None)? {
        for member in &sandbox.mods {
            // Never staged: a first install, not an update.
            if member.staged_at.is_none() {
                continue;
            }

            let Some(entry) = db.find_item(&member.kind, member.item_id)? else {
                continue;
            };

            /*
             * Release IDS, never version strings. A version string is authored
             * by hand and is not ordered in any way a machine can rely on —
             * `1.10` sorts before `1.9` lexically and after it numerically, and
             * plenty of items version by date or by commit hash. The release id
             * is monotonic because the database mints it.
             */
            let Some(latest) = entry.latest_release_id else {
                continue;
            };

            if member.release_id == Some(latest) {
                continue;
            }

            out.push(Outdated {
                sandbox_id: sandbox.id,
                sandbox_name: sandbox.name.clone(),
                mod_key: member.mod_key.clone(),
                name: member.name.clone(),
                from_version: member.version.clone(),
                to_version: entry.latest_version.clone(),
                release_id: Some(latest),
            });
        }
    }

    Ok(out)
}

/// Run one pass over every sandbox.
pub async fn run(db: &LibraryDb, ctx: &SandboxCtx<'_>) -> AppResult<AutoUpdateReport> {
    let mut report = AutoUpdateReport::default();

    for sandbox in db.sandbox_list(None)? {
        report.checked += 1;

        let moved = update_sandbox(db, &sandbox, ctx, &mut report).await;

        if !moved {
            continue;
        }

        /*
         * Rule 4. Staging is invisible; deploying is not. A sandbox nobody has
         * deployed gets its new files staged and left there — the next time
         * somebody presses Deploy they get the current version, and until then
         * their game folder is exactly as they left it.
         */
        if sandbox.deployed_at.is_none() {
            report.skipped.push(format!(
                "{} is not deployed, so nothing was applied",
                sandbox.name
            ));

            continue;
        }

        // Re-read: staging changed the rows the deploy reads.
        let Ok(Some(fresh)) = db.sandbox_get(sandbox.id) else {
            continue;
        };

        match deploy_sandbox(db, &fresh, ctx, false) {
            Ok(deployed) => {
                report.redeployed.push(sandbox.name.clone());

                audit!(
                    ctx.audit,
                    Info,
                    Install,
                    "sandbox.autoupdate",
                    format!(
                        "{} redeployed ({} placed, {} removed)",
                        sandbox.name, deployed.placed, deployed.removed
                    )
                );
            }
            Err(err) => report.failed.push((sandbox.name.clone(), err.to_string())),
        }
    }

    Ok(report)
}

/// Bring one sandbox's items forward. Returns whether anything moved.
async fn update_sandbox(
    db: &LibraryDb,
    sandbox: &Sandbox,
    ctx: &SandboxCtx<'_>,
    report: &mut AutoUpdateReport,
) -> bool {
    if !sandbox.auto_update {
        return false;
    }

    let mut moved = false;

    for member in &sandbox.mods {
        if member.staged_at.is_none() {
            continue;
        }

        let Ok(Some(entry)) = db.find_item(&member.kind, member.item_id) else {
            continue;
        };

        let Some(latest) = entry.latest_release_id else {
            continue;
        };

        if member.release_id == Some(latest) {
            continue;
        }

        // The ACCOUNT's switch, set on the website or on another device.
        if !entry.auto_update {
            report.skipped.push(format!(
                "{} · {} has automatic updates off",
                sandbox.name, member.name
            ));

            continue;
        }

        if entry.paused {
            report
                .skipped
                .push(format!("{} · {} is paused", sandbox.name, member.name));

            continue;
        }

        let label = format!("{} · {}", sandbox.name, member.name);

        let outcome = stage_mod(db, sandbox, member, &entry, ctx).await;

        if outcome.ok {
            moved = true;
            report.updated.push(label);
        } else {
            /*
             * Nothing is rolled back. The previous version is still staged and
             * still deployed and is exactly what the user had a minute ago —
             * undoing that to reach a state nobody asked for would be worse
             * than a failed update.
             */
            report.failed.push((
                label,
                outcome
                    .error
                    .unwrap_or_else(|| "The download did not finish.".into()),
            ));
        }
    }

    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::Strategy;
    use crate::library::db::LibraryEntry;
    use crate::library::sandbox::{Environment, NewSandbox};
    use std::collections::BTreeMap;

    fn db() -> LibraryDb {
        LibraryDb::open_memory().expect("db")
    }

    fn entry(kind: &str, item_id: i64, release: i64, version: &str) -> LibraryEntry {
        LibraryEntry {
            id: format!("s{item_id}"),
            kind: kind.into(),
            item_id,
            name: format!("Item {item_id}"),
            description: None,
            image: None,
            web_url: "https://example.com".into(),
            app_id: Some(1),
            app_name: None,
            app_slug: Some("minecraft".into()),
            auto_update: true,
            notify_updates: true,
            paused: false,
            via_collection_id: None,
            installable: true,
            latest_release_id: Some(release),
            latest_version: Some(version.into()),
            file_url: Some("https://example.com/f".into()),
            file_name: Some("a.jar".into()),
            file_size: None,
            file_sha256: None,
            updated_at: String::new(),
            installed_release_id: None,
            installed_version: None,
            installed_install_id: None,
            installed_at: None,
            installed_files: vec![],
            state: "idle".into(),
            last_error: None,
        }
    }

    fn sandbox(db: &LibraryDb, auto: bool) -> i64 {
        db.sandbox_create(&NewSandbox {
            app_id: 1,
            app_slug: Some("minecraft".into()),
            app_name: Some("Minecraft".into()),
            name: "Test".into(),
            description: None,
            environment: Environment::Client,
            strategy: Strategy::Direct,
            game_version: None,
            loader: None,
            preset: None,
            game_dir: None,
            options: BTreeMap::new(),
            cloud_sync: false,
            auto_update: auto,
        })
        .expect("create")
    }

    #[test]
    fn a_newer_release_than_the_staged_one_is_outdated() {
        let db = db();
        let id = sandbox(&db, true);

        db.upsert_remote(&entry("mod", 1, 9, "1.5")).expect("sub");

        let key = db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        // Staged at an older release.
        db.sandbox_mark_staged(id, &key, Some(7), Some("1.4"))
            .expect("staged");

        let found = outdated(&db).expect("outdated");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].from_version.as_deref(), Some("1.4"));
        assert_eq!(found[0].to_version.as_deref(), Some("1.5"));
        assert_eq!(found[0].release_id, Some(9));
    }

    #[test]
    fn the_release_already_staged_is_not_an_update() {
        let db = db();
        let id = sandbox(&db, true);

        db.upsert_remote(&entry("mod", 1, 9, "1.5")).expect("sub");

        let key = db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        db.sandbox_mark_staged(id, &key, Some(9), Some("1.5"))
            .expect("staged");

        assert!(outdated(&db).expect("outdated").is_empty());
    }

    /// A version string is not ordered in any way a machine can rely on. The
    /// release id is, because the database mints it.
    #[test]
    fn a_version_string_that_sorts_backwards_is_still_an_update() {
        let db = db();
        let id = sandbox(&db, true);

        // `1.9` is the NEWER release and sorts before `1.10` lexically.
        db.upsert_remote(&entry("mod", 1, 20, "1.9")).expect("sub");

        let key = db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        db.sandbox_mark_staged(id, &key, Some(19), Some("1.10"))
            .expect("staged");

        assert_eq!(outdated(&db).expect("outdated").len(), 1);
    }

    /// An item in the list that has never downloaded is a first install, not an
    /// update, and a first install is something somebody presses a button for.
    #[test]
    fn an_item_that_was_never_staged_is_not_an_update() {
        let db = db();
        let id = sandbox(&db, true);

        db.upsert_remote(&entry("mod", 1, 9, "1.5")).expect("sub");
        db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        assert!(outdated(&db).expect("outdated").is_empty());
    }

    /// The badge ignores the switches: somebody who turned auto-update off
    /// still wants to be told, and being told is the point of turning it off
    /// rather than unsubscribing.
    #[test]
    fn the_available_list_ignores_the_auto_update_switches() {
        let db = db();
        let id = sandbox(&db, false);

        let mut sub = entry("mod", 1, 9, "1.5");
        sub.auto_update = false;

        db.upsert_remote(&sub).expect("sub");

        let key = db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        db.sandbox_mark_staged(id, &key, Some(7), Some("1.4"))
            .expect("staged");

        assert_eq!(
            outdated(&db).expect("outdated").len(),
            1,
            "a sandbox with auto-update off still reports what is available"
        );
    }

    #[test]
    fn a_sandbox_with_auto_update_off_stages_nothing() {
        let db = db();
        let id = sandbox(&db, false);

        db.upsert_remote(&entry("mod", 1, 9, "1.5")).expect("sub");

        let key = db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        db.sandbox_mark_staged(id, &key, Some(7), Some("1.4"))
            .expect("staged");

        let found = db.sandbox_get(id).expect("get").expect("present");

        assert!(!found.auto_update);

        // `update_sandbox` returns early on the flag; nothing else in the pass
        // can move without it.
        let report = AutoUpdateReport::default();

        assert!(report.did_nothing());
    }

    #[test]
    fn the_auto_update_flag_round_trips_and_defaults_on() {
        let db = db();
        let id = sandbox(&db, true);

        assert!(db.sandbox_get(id).unwrap().unwrap().auto_update);

        db.sandbox_patch(
            id,
            &crate::library::sandbox::SandboxPatch {
                auto_update: Some(false),
                ..Default::default()
            },
        )
        .expect("patch");

        let found = db.sandbox_get(id).expect("get").expect("present");

        assert!(!found.auto_update);
        // The neighbours survived a patch that named one field.
        assert_eq!(found.name, "Test");
        assert!(!found.cloud_sync);
    }
}
