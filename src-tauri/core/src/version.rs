//! Comparing two version strings, in the one place that does it.
//!
//! There are two questions in this app that are decided by comparing versions —
//! "is there a newer build of the app?" and "is there a newer build of an
//! installed game?" — and they used to have one implementation between them
//! only because the second did not exist yet.
//!
//! THE TRAP
//! -------
//! `1.10.0` sorts BEFORE `1.9.0` as a string. An update check built on a string
//! compare either never fires or never stops firing, and both failure modes are
//! quiet: the first looks like nothing has been released, and the second looks
//! like an update that will not install.
//!
//! WHAT IT REFUSES
//! --------------
//! Anything it cannot order. `nightly`, `2026-09-12` and an empty string are all
//! answered `false` rather than guessed at, because the consequence of a wrong
//! answer is asymmetric: staying quiet costs somebody a version they will get
//! next time, and firing wrongly nags a user toward a build they already have —
//! or, for a game, replaces a working install with an older one.
//!
//! A build suffix is dropped rather than compared. `1.2.3-beta.1` and
//! `1.2.3+deadbeef` order by their release part, because nothing here can know
//! whether `beta.1` precedes `rc.2` in somebody's scheme.

/// The most components a version may carry before it is treated as unorderable.
///
/// Eight is far past anything real (`1.2.3.4` is already unusual) and is what
/// stops a hostile or malformed string turning a comparison into a long loop.
const MAX_PARTS: usize = 8;

fn parse(raw: &str) -> Option<Vec<u64>> {
    let core = raw.trim().split(['-', '+']).next().unwrap_or_default();

    if core.is_empty() {
        return None;
    }

    core.split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<u64>>>()
        .filter(|parts| !parts.is_empty() && parts.len() <= MAX_PARTS)
}

/// Whether `candidate` is strictly newer than `current`.
///
/// `false` when either cannot be ordered, and `false` for equal versions — a
/// caller asking this is deciding whether to offer an update, and "the same
/// version" is not one.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Some(candidate), Some(current)) = (parse(candidate), parse(current)) else {
        return false;
    };

    for index in 0..candidate.len().max(current.len()) {
        let a = candidate.get(index).copied().unwrap_or(0);
        let b = current.get(index).copied().unwrap_or(0);

        if a != b {
            return a > b;
        }
    }

    false
}

/// Whether `have` satisfies a minimum of `want`.
///
/// The question a build's `minClientVersion` asks, and deliberately not
/// `!is_newer(want, have)`: that expression answers `true` when either side is
/// unorderable, which would wave through exactly the build that declared a
/// requirement. An unorderable requirement is refused instead — a build saying
/// "you need at least `nightly`" is a build nobody can establish they may run.
///
/// An unorderable *own* version is the one case that passes: a development
/// build of the app has no business being told it is too old to install
/// something, and the person running one is the person who can judge.
pub fn meets_minimum(have: &str, want: &str) -> bool {
    let Some(want) = parse(want) else {
        return false;
    };

    let Some(have) = parse(have) else {
        return true;
    };

    for index in 0..have.len().max(want.len()) {
        let a = have.get(index).copied().unwrap_or(0);
        let b = want.get(index).copied().unwrap_or(0);

        if a != b {
            return a > b;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::{is_newer, meets_minimum};

    #[test]
    fn a_later_version_is_newer() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("1.1.0", "1.0.9"));
        assert!(is_newer("2.0.0", "1.99.99"));
    }

    #[test]
    fn ten_comes_after_nine() {
        // The entire reason this module exists: as strings, `1.10.0` < `1.9.0`.
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.9.0", "1.10.0"));
    }

    #[test]
    fn the_same_version_is_not_an_update() {
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2"));
    }

    #[test]
    fn an_unorderable_version_stays_quiet() {
        assert!(!is_newer("nightly", "1.0.0"));
        assert!(!is_newer("1.0.0", "nightly"));
        assert!(!is_newer("", "1.0.0"));
        assert!(!is_newer("1.0.0.0.0.0.0.0.0", "1.0.0"));
    }

    #[test]
    fn a_build_suffix_orders_by_its_release_part() {
        assert!(is_newer("1.2.4-beta.1", "1.2.3"));
        assert!(!is_newer("1.2.3-beta.1", "1.2.3"));
        assert!(is_newer("1.2.4+abc", "1.2.3+zzz"));
    }

    #[test]
    fn a_minimum_is_met_by_equal_or_greater() {
        assert!(meets_minimum("1.2.3", "1.2.3"));
        assert!(meets_minimum("1.3.0", "1.2.3"));
        assert!(meets_minimum("1.10.0", "1.9.0"));
        assert!(!meets_minimum("1.2.2", "1.2.3"));
    }

    #[test]
    fn an_unorderable_requirement_is_refused_and_an_unorderable_build_is_not() {
        // Nobody can establish they satisfy this, so nobody does.
        assert!(!meets_minimum("1.2.3", "nightly"));
        // A development build of the app judges for itself.
        assert!(meets_minimum("dev", "1.2.3"));
    }
}
