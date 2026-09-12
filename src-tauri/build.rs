fn main() {
    /*
     * The updater's public key is compiled in, so a rebuild has to notice when
     * it changes.
     *
     * Without this, a build that was first made with no key keeps having no key
     * forever — cargo does not know an `option_env!` was read — and the symptom
     * is an app that silently has no updater after somebody has configured one.
     * The same reasoning `core/build.rs` carries for `TMC_API_BASE`.
     */
    println!("cargo:rerun-if-env-changed=TMC_UPDATER_PUBKEY");

    tauri_build::build()
}
