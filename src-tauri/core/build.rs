//! One job: make `option_env!("TMC_API_BASE")` honest.
//!
//! Cargo does not track the environment a crate was compiled in unless it is
//! told to, so without this line changing `TMC_API_BASE` and rebuilding gives
//! you the PREVIOUS base baked into an up-to-date-looking binary — the app
//! quietly keeps talking to whichever site it was first built against, and the
//! only cure is `cargo clean`.

fn main() {
    println!("cargo:rerun-if-env-changed=TMC_API_BASE");
}
