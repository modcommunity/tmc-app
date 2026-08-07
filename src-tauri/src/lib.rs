mod commands;
mod paths;
mod state;

use tauri::Manager;

use tmc_core::audit;

use crate::state::AppState;

/// Cache is scratch space for in-flight downloads. Clearing it on launch means
/// a crash mid-install cannot leave a half-written archive that a later run
/// picks up as if it were complete.
fn clear_cache(paths: &paths::AppPaths) {
    let downloads = paths.cache.join("downloads");

    if downloads.exists() {
        let _ = std::fs::remove_dir_all(&downloads);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tmc_app_lib=info,tmc_core=info,warn".into()),
        )
        .init();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init());

    /*
     * Deep links accelerate the login: the browser bounces to `tmc://auth`
     * after approval so the app polls immediately instead of waiting out its
     * interval. Registration is desktop-only here — on Android and iOS the
     * scheme is declared in the platform manifests that `tauri android init` /
     * `tauri ios init` generate, and calling the desktop registrar there is
     * both unnecessary and unsupported.
     *
     * The link is a WAKE-UP, never a credential. Nothing is read out of the
     * URL; the app polls the API exactly as it would have on its timer. That is
     * deliberate — a custom scheme can be claimed by any other app on the
     * machine, so anything carried in one is public.
     *
     * Shadowed rather than reassigned, so `builder` needs no `mut` on the
     * mobile targets where this is compiled out.
     */
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_deep_link::init());

    builder
        .setup(|app| {
            let handle = app.handle().clone();
            let version = app.package_info().version.to_string();

            let state = AppState::build(&handle, version)
                .map_err(|e| format!("could not start: {}", e.detail()))?;

            clear_cache(&state.paths);

            audit!(
                state.audit,
                Info,
                App,
                "app.start",
                format!("TMC {} started", state.version)
            );

            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;

                // Best effort: a failed registration means the accelerator does
                // not work, not that the app cannot sign in.
                let _ = app.deep_link().register("tmc");

                let emitter = app.handle().clone();

                app.deep_link().on_open_url(move |_event| {
                    use tauri::Emitter;

                    let _ = emitter.emit("tmc://auth-return", ());
                });
            }

            app.manage(state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::session_state,
            commands::auth::auth_begin,
            commands::auth::auth_poll,
            commands::auth::auth_cancel,
            commands::auth::auth_sign_out,
            commands::api::api_get,
            commands::api::api_send,
            commands::settings::settings_get,
            commands::settings::settings_patch,
            commands::settings::settings_reset,
            commands::logs::log_read,
            commands::logs::log_clear,
            commands::logs::log_path,
            commands::servers::ping_server,
            commands::servers::query_server,
            commands::servers::query_servers,
            commands::servers::latency_series,
            commands::servers::latency_clear,
            commands::plugins::plugin_list,
            commands::plugins::plugin_inspect,
            commands::plugins::plugin_approve,
            commands::plugins::plugin_set_enabled,
            commands::plugins::plugin_remove,
            commands::plugins::plugin_theme,
            commands::plugins::plugin_run,
            commands::plugins::plugin_query_server,
            commands::library::library_list,
            commands::library::library_sync,
            commands::library::library_install,
            commands::library::library_uninstall,
            commands::library::library_installs,
            commands::library::library_install_dirs,
            commands::library::library_set_install_dir,
            commands::library::library_supported_apps,
            commands::library::library_plugin_errors,
            commands::library::library_reload_plugins,
            commands::library::library_rules_for,
            commands::library::launch_preview,
            commands::library::launch_install,
            commands::library::launch_available,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
