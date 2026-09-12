//! Keeping installed games current.
//!
//! The games half of `library::autoupdate`, and it follows the same shape: one
//! function that REPORTS what has moved and one that acts, with the acting one
//! bounded by rules that are written down rather than implied.
//!
//! THE RULES
//! ---------
//!   1. **The install must have `auto_update` on.** Per game rather than
//!      global, because the split is the useful case: a game somebody plays
//!      with friends tracks whatever the servers are running, and one they are
//!      mid-way through does not need to change under them tonight.
//!   2. **A game that is RUNNING is never touched.** The new files would land
//!      on top of ones a live process has mapped — which Windows refuses
//!      outright, and which Unix permits, leaving a process running code that
//!      is no longer on disk. The game is skipped and caught by the next pass.
//!   3. **Only an install that already exists is updated.** An app with a build
//!      the device has never installed is not an update; it is a first install,
//!      which is a thing somebody presses a button for.
//!   4. **A version that cannot be ordered is not an update.** `version::is_newer`
//!      refuses what it cannot compare, so a build published as `nightly` never
//!      presents as newer than `1.2.0` — and never replaces it.
//!
//! WHAT A FAILURE DOES
//! ------------------
//! Nothing, and it says so. One game that will not download leaves every other
//! game alone and leaves that game at the version it already had — the previous
//! version is still on disk and still works, because the new one lands in its
//! own directory and the row is only repointed once it is verified.

use serde::Serialize;

use crate::audit;
use crate::error::AppResult;
use crate::version::is_newer;

use super::{GameStatus, Games};

/// What one pass did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameUpdateReport {
    /// Games moved to a newer build.
    pub updated: Vec<String>,
    /// Games that had one available and were left alone, with the reason.
    pub skipped: Vec<String>,
    /// Games whose update failed, with the message.
    pub failed: Vec<String>,
}

impl GameUpdateReport {
    pub fn did_nothing(&self) -> bool {
        self.updated.is_empty() && self.skipped.is_empty() && self.failed.is_empty()
    }
}

/// Ask the site about every installed game, and say which have moved.
///
/// ONE REQUEST PER INSTALLED GAME, which is the right shape here and would not
/// be for subscriptions: a device holds a handful of games and hundreds of
/// mods, and the mod answer already rides along on the library's own sync. A
/// batch endpoint for this would be an endpoint existing to serve a list nobody
/// has.
///
/// A game whose lookup fails is reported at its current version rather than
/// dropped: "we could not ask" and "there is no update" are different facts,
/// and a Library that silently omitted the first would show a stale game as
/// current.
pub async fn check(games: &Games) -> AppResult<Vec<GameStatus>> {
    let mut out = Vec::new();

    for installed in games.installed()? {
        let available = games.resolve(installed.app_id).await.ok().flatten();

        let update_available = available
            .as_ref()
            .is_some_and(|build| is_newer(&build.version, &installed.version));

        out.push(GameStatus {
            available: available.as_ref().map(|b| b.version.clone()),
            notes: available.and_then(|b| b.notes),
            update_available,
            installed,
        });
    }

    Ok(out)
}

/// Update every installed game that asked to be kept current.
///
/// Driven by the library's sync pass, which is the device's own "am I online
/// now?" heartbeat — the same place `sessions_flush` hangs off, and for the same
/// reason: it is already the moment the device has an API to talk to.
///
/// `running` is a closure rather than a `Sessions` handle because `tmc-core`
/// does not get to depend on the shell's idea of what a session is — and
/// `Sync` is on it because the future this returns is held across an `await`
/// inside a Tauri command, which is spawned onto a work-stealing runtime.
pub async fn run(
    games: &Games,
    running: &(dyn Fn(i64) -> bool + Send + Sync),
) -> AppResult<GameUpdateReport> {
    let mut report = GameUpdateReport::default();

    for status in check(games).await? {
        if !status.update_available {
            continue;
        }

        let row = &status.installed;

        if !row.auto_update {
            report
                .skipped
                .push(format!("{}: automatic updates are off", row.name));

            continue;
        }

        if running(row.app_id) {
            report.skipped.push(format!("{}: it is running", row.name));

            continue;
        }

        match games
            .install(row.app_id, &row.name, row.slug.as_deref())
            .await
        {
            Ok(updated) => report
                .updated
                .push(format!("{} → {}", updated.name, updated.version)),
            Err(error) => report.failed.push(format!("{}: {error}", row.name)),
        }
    }

    if !report.did_nothing() {
        audit!(
            games.audit,
            Info,
            App,
            "game.autoupdate",
            format!(
                "{} updated, {} skipped, {} failed",
                report.updated.len(),
                report.skipped.len(),
                report.failed.len()
            )
        );
    }

    Ok(report)
}
