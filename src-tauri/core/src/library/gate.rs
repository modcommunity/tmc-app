//! One install or uninstall at a time per subscription.
//!
//! Two sync passes (the timer and "Sync now"), or an unsubscribe landing while the file is still
//! downloading, would otherwise run on the same files at once: the uninstall removes nothing
//! because the install has not recorded its files yet, and the install then finishes and
//! strands them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

static GATES: OnceLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();

/// Wait for this subscription's turn. The flag says whether another run held it first, in which
/// case the caller's copy of the row is stale and must be re-read.
pub async fn enter(id: &str) -> (OwnedMutexGuard<()>, bool) {
    let gate = {
        let mut gates = GATES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        Arc::clone(gates.entry(id.to_string()).or_default())
    };

    match Arc::clone(&gate).try_lock_owned() {
        Ok(guard) => (guard, false),
        Err(_) => (gate.lock_owned().await, true),
    }
}

/// What an install should do once it holds the gate.
#[derive(Debug, PartialEq, Eq)]
pub enum Turn {
    Go,
    /// The row was deleted (unsubscribed) while this run waited.
    Gone,
    /// The run ahead already installed this very release.
    AlreadyDone,
}

/// `now` is the row's (installed, latest) release ids, or `None` when the row is gone.
pub fn install_turn(waited: bool, now: Option<(Option<i64>, Option<i64>)>) -> Turn {
    match now {
        None => Turn::Gone,
        Some((installed, latest)) if waited && installed.is_some() && installed == latest => {
            Turn::AlreadyDone
        }
        Some(_) => Turn::Go,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_second_run_for_the_same_id_waits_and_other_ids_do_not() {
        let (first, waited) = enter("gate-a").await;

        assert!(!waited);

        let (other, other_waited) = enter("gate-b").await;

        assert!(!other_waited);
        drop(other);

        let second = tokio::spawn(async { enter("gate-a").await.1 });

        tokio::task::yield_now().await;
        assert!(!second.is_finished());

        drop(first);

        assert!(second.await.expect("join"));
    }

    #[test]
    fn a_run_that_waited_skips_work_the_one_ahead_finished_and_stops_for_a_deleted_row() {
        assert_eq!(install_turn(true, None), Turn::Gone);
        assert_eq!(install_turn(true, Some((Some(7), Some(7)))), Turn::AlreadyDone);
        assert_eq!(install_turn(true, Some((Some(6), Some(7)))), Turn::Go);
        // A user asking to reinstall an up to date item, with nobody ahead of them, still runs.
        assert_eq!(install_turn(false, Some((Some(7), Some(7)))), Turn::Go);
    }
}
