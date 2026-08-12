//! `TMC_API_BASE` read at run time.
//!
//! An integration test rather than a unit one because it needs a process of its
//! own: `api_base()` resolves into a `OnceLock` on first use, so a test that set
//! the variable would be racing every other test in the binary for who calls it
//! first. One test, one binary, no ordering to get wrong.

#[test]
fn the_environment_moves_the_base_and_its_stored_session() {
    // The override is compiled out of a release build on purpose — see
    // `RUNTIME_OVERRIDE` in `api.rs`. `cargo test --release` must not fail for
    // doing what it was told.
    if !cfg!(debug_assertions) {
        return;
    }

    std::env::set_var("TMC_API_BASE", "https://tmcdev.net:3002/");

    assert_eq!(tmc_core::api::api_base(), "https://tmcdev.net:3002");
    assert!(!tmc_core::api::api_base_is_prod());

    // The refresh token lands under its own key, so a dev run cannot read or
    // clobber the production session.
    assert_eq!(
        tmc_core::api::api_base_scope().as_deref(),
        Some("tmcdev-net-3002")
    );
}
