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

/// What a pass has decided to do about one installed game.
///
/// Split out of [`run`] so the four rules in this module's header are
/// CHECKABLE. They were argued at length and enforced inside a loop that needs
/// a database, an API client and a download manager to enter, which meant the
/// only way to verify "a game that is running is never touched" was to read it
/// and agree. A rule nothing can fail is a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateDecision {
    /// There is nothing newer, so there is nothing to say. Not reported —
    /// a Library listing every current game as "skipped" is a report nobody
    /// can read.
    Leave,
    /// Something newer exists and a rule stopped it. The string is what the
    /// report shows, so it names the game and says which rule.
    Skip(String),
    /// Install it.
    Update,
}

/// Apply the rules to one game, in the order their failure is most useful.
///
/// `auto_update` BEFORE `running`, and the order is load-bearing rather than
/// arbitrary: a game with automatic updates switched off is skipped for that
/// reason whether or not it happens to be running at this moment, and the
/// opposite order would report a stable preference as a transient condition —
/// "it is running" invites somebody to close it and try again, which would
/// then change nothing.
pub fn decide(status: &GameStatus, running: bool) -> UpdateDecision {
    if !status.update_available {
        return UpdateDecision::Leave;
    }

    let row = &status.installed;

    if !row.auto_update {
        return UpdateDecision::Skip(format!("{}: automatic updates are off", row.name));
    }

    if running {
        return UpdateDecision::Skip(format!("{}: it is running", row.name));
    }

    UpdateDecision::Update
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
        let row = &status.installed;

        match decide(&status, running(row.app_id)) {
            UpdateDecision::Leave => continue,
            UpdateDecision::Skip(reason) => {
                report.skipped.push(reason);

                continue;
            }
            UpdateDecision::Update => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::db::InstalledGame;

    /// One installed game. Everything the rules do not read is fixed, so a
    /// test reads as the one fact it is about.
    fn installed(name: &str, version: &str, auto_update: bool) -> InstalledGame {
        InstalledGame {
            app_id: 42,
            slug: Some("arena".into()),
            name: name.into(),
            platform: "LINUX_X64".into(),
            version: version.into(),
            dir: "/games/arena/1.0.0".into(),
            entry: Some("game.x86_64".into()),
            args: Vec::new(),
            size_bytes: 512,
            installed_ms: 0,
            updated_ms: 0,
            auto_update,
        }
    }

    /// A status as `check` would have built it — `update_available` computed
    /// by `is_newer` rather than asserted, so these tests exercise the same
    /// comparison the real pass does.
    fn status(installed: InstalledGame, available: Option<&str>) -> GameStatus {
        let update_available = available.is_some_and(|v| is_newer(v, &installed.version));

        GameStatus {
            available: available.map(str::to_string),
            notes: None,
            update_available,
            installed,
        }
    }

    #[test]
    fn a_newer_build_on_an_auto_updating_game_is_installed() {
        let s = status(installed("Arena", "1.0.0", true), Some("1.1.0"));

        assert!(s.update_available);
        assert_eq!(decide(&s, false), UpdateDecision::Update);
    }

    #[test]
    fn nothing_newer_is_left_alone_and_not_reported() {
        // Same version, and a strictly OLDER one. Neither is an update, and
        // the second is the case that matters: a rollback published by
        // mistake must not be installed as though it were an upgrade.
        for available in ["1.0.0", "0.9.0"] {
            let s = status(installed("Arena", "1.0.0", true), Some(available));

            assert!(!s.update_available, "{available} should not be an update");
            assert_eq!(decide(&s, false), UpdateDecision::Leave);
        }
    }

    #[test]
    fn a_version_that_cannot_be_ordered_is_never_an_update() {
        // Rule 4. `nightly` is not after `1.2.0` — it is not anywhere — and a
        // build published under a name like that must not replace a release.
        for available in ["nightly", "main", "", "latest"] {
            let s = status(installed("Arena", "1.2.0", true), Some(available));

            assert!(
                !s.update_available,
                "{available:?} should not present as newer than 1.2.0"
            );
            assert_eq!(decide(&s, false), UpdateDecision::Leave);
        }
    }

    #[test]
    fn ten_is_after_nine_rather_than_before_it() {
        // The reason `version::is_newer` exists at all. A string compare puts
        // "1.10.0" before "1.9.0", which would pin every game on the ninth
        // point release for ever and report nothing wrong.
        let s = status(installed("Arena", "1.9.0", true), Some("1.10.0"));

        assert!(s.update_available);
        assert_eq!(decide(&s, false), UpdateDecision::Update);
    }

    #[test]
    fn a_game_with_automatic_updates_off_is_skipped_with_that_reason() {
        let s = status(installed("Arena", "1.0.0", false), Some("1.1.0"));

        assert_eq!(
            decide(&s, false),
            UpdateDecision::Skip("Arena: automatic updates are off".into())
        );
    }

    #[test]
    fn a_running_game_is_never_touched() {
        // Rule 2, and the one with a real cost behind it: the new files would
        // land on top of ones a live process has mapped.
        let s = status(installed("Arena", "1.0.0", true), Some("1.1.0"));

        assert_eq!(
            decide(&s, true),
            UpdateDecision::Skip("Arena: it is running".into())
        );
    }

    #[test]
    fn off_beats_running_when_both_apply() {
        // The order in `decide` is load-bearing. "It is running" invites
        // somebody to close the game and try again; with automatic updates off
        // that would change nothing, so the stable reason is the one to give.
        let s = status(installed("Arena", "1.0.0", false), Some("1.1.0"));

        assert_eq!(
            decide(&s, true),
            UpdateDecision::Skip("Arena: automatic updates are off".into())
        );
    }

    #[test]
    fn a_game_the_site_could_not_be_asked_about_is_left_alone() {
        // `check` reports a failed lookup as `available: None` rather than
        // dropping the game. "We could not ask" must not act like "there is
        // something newer".
        let s = status(installed("Arena", "1.0.0", true), None);

        assert!(!s.update_available);
        assert_eq!(decide(&s, false), UpdateDecision::Leave);
    }

    #[test]
    fn an_empty_report_did_nothing_and_any_entry_makes_it_something() {
        assert!(GameUpdateReport::default().did_nothing());

        for report in [
            GameUpdateReport {
                updated: vec!["a".into()],
                ..Default::default()
            },
            GameUpdateReport {
                skipped: vec!["a".into()],
                ..Default::default()
            },
            GameUpdateReport {
                failed: vec!["a".into()],
                ..Default::default()
            },
        ] {
            assert!(
                !report.did_nothing(),
                "a report with an entry in it did something: {report:?}"
            );
        }
    }
}
