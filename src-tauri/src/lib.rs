mod commands;
mod paths;
mod spawn;
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

/// Routes each recognised link to the webview, or holds it until the webview asks.
///
/// An unrecognised link does nothing: any program can claim a custom scheme.
#[cfg(desktop)]
fn dispatch<'a>(app: &tauri::AppHandle, urls: impl Iterator<Item = &'a str>) {
    use tauri::Emitter;
    use tmc_core::deeplink::{parse, DeepLink, PendingLink};

    for url in urls {
        let Some(link) = parse(url) else {
            tracing::debug!("ignoring unrecognised deep link");

            continue;
        };

        if let DeepLink::Auth = link {
            let _ = app.emit("tmc://auth-return", ());

            continue;
        }

        let delivery = app.state::<PendingLink>().offer(link);

        let _ = app.emit("tmc://open", &delivery);
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

    /*
     * `tauri-plugin-dialog` is deliberately NOT registered.
     *
     * Its folder picker is the platform's, which on Linux is a GTK file chooser
     * — themed by whatever desktop the user is running and matching neither the
     * app nor the other four targets. Folder choosing is `components/
     * folder-picker` over `commands::fs`, which looks the same everywhere; see
     * that module's header for what listing directories from the webview costs.
     */
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init());

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
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_deep_link::init());

    /*
     * SELF-UPDATING. The plugin is ALWAYS registered on desktop; whether an
     * update can be INSTALLED is decided per call.
     *
     * The plugin's entire security model is that it verifies a minisign
     * signature against a public key before it installs anything. Which key,
     * and which endpoint, are resolved on every check by
     * `tmc_core::updater::resolve` — the Settings override, else the key and
     * endpoint compiled in (`TMC_UPDATER_PUBKEY`, which `build.rs` fills from
     * `updater.pub`, and `TMC_UPDATER_ENDPOINT`), else no key at all — and
     * handed to `updater_builder().pubkey(..)` there. A build with no key
     * anywhere refuses to install rather than trusting what it downloads, and
     * says so in Settings; it never crashes and never checks unsigned.
     *
     * Registering unconditionally is what lets a key supplied at RUNTIME work:
     * `updater_builder()` reads the plugin's managed state and panics without
     * it. `tauri.conf.json` carries an empty `plugins.updater` block because the
     * plugin's config needs a `pubkey` field to deserialise; nothing reads it.
     */
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

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

            /*
             * A build aimed at anything but the real site says so in the audit
             * trail, at Security level so turning logging off cannot hide it.
             * Where an account's credentials are being sent is exactly the kind
             * of fact the log exists to hold, and "which site was this?" is the
             * first question about any screenshot from a dev build.
             */
            if !tmc_core::api::api_base_is_prod() {
                audit!(
                    state.audit,
                    Security,
                    App,
                    "app.api_base",
                    format!("Talking to {} — NOT production", tmc_core::api::api_base())
                );
            }

            #[cfg(desktop)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;

                // Best effort: a failed registration means the accelerator does
                // not work, not that the app cannot sign in.
                let _ = app.deep_link().register("tmc");

                let emitter = app.handle().clone();

                app.manage(tmc_core::deeplink::PendingLink::default());

                app.deep_link().on_open_url(move |event| {
                    dispatch(&emitter, event.urls().iter().map(|u| u.as_str()));
                });

                // A cold start from a link: the plugin has already seen it.
                if let Ok(Some(urls)) = app.deep_link().get_current() {
                    dispatch(app.handle(), urls.iter().map(|u| u.as_str()));
                }
            }

            app.manage(state);

            /*
             * Files dropped on the window are handled HERE, in Rust, and never
             * as a path handed to the webview.
             *
             * Tauri delivers the drop to the window, which means the paths pass
             * through this process before anything else sees them. So the
             * frontend is given names and opaque tokens — see
             * `tmc_core::local::vault` — and `commands::import` is the only
             * thing that ever resolves one back to a path. That is what keeps
             * `commands/mod.rs`'s "no command takes a path from the webview"
             * rule true with drag and drop in the app.
             *
             * `dragDropEnabled` is ON in `tauri.conf.json` for exactly this. It
             * also means the webview receives no HTML5 drag events, which is
             * fine and is the point: an `ondrop` handler in the webview would
             * be the untrusted half deciding what was dropped.
             */
            //
            // The main window only, and by construction rather than by a label
            // check: the web player's window is a remote page with no IPC to
            // tell about a drop anyway.
            if let Some(window) = app.get_webview_window("main") {
                use tauri::{DragDropEvent, Emitter, WindowEvent};

                let handle = app.handle().clone();

                window.on_window_event(move |event| {
                    let WindowEvent::DragDrop(drag) = event else {
                        return;
                    };

                    /*
                     * The hover half is emitted too, and has to be: with
                     * `dragDropEnabled` on, the webview receives NO HTML5 drag
                     * events at all — which is the point, since an `ondrop`
                     * handler there would be the untrusted half deciding what
                     * was dropped. Without these two the feature would have no
                     * affordance whatsoever: the window would simply accept a
                     * file with nothing on screen having suggested it would.
                     *
                     * They carry a count, never a name and never a path. What
                     * is being dragged is not yet the app's business.
                     */
                    let paths = match drag {
                        DragDropEvent::Enter { paths, .. } => {
                            let _ = handle.emit("tmc://files-drag", paths.len());

                            return;
                        }
                        DragDropEvent::Leave => {
                            let _ = handle.emit("tmc://files-drag", 0usize);

                            return;
                        }
                        DragDropEvent::Drop { paths, .. } => paths,
                        _ => return,
                    };

                    let _ = handle.emit("tmc://files-drag", 0usize);

                    let state = handle.state::<AppState>();

                    let batch = commands::import::batch_from_drop(&state, paths.clone());

                    if batch.files.is_empty() {
                        return;
                    }

                    audit!(
                        state.audit,
                        Info,
                        App,
                        "import.drop",
                        format!("{} file(s) dropped on the window", batch.files.len())
                    );

                    state.set_drop(batch.clone());

                    let _ = handle.emit("tmc://files-dropped", &batch);
                });
            }

            /*
             * The download queue's bridge to the webview, and the queue itself
             * restored from the last session. Both after `manage`, because both
             * reach the state through the handle.
             */
            commands::downloads::spawn_bridge(app.handle().clone());

            let restore_handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                use tauri::Manager;

                commands::downloads::restore(&restore_handle.state::<AppState>()).await;
            });

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
            commands::api::api_env,
            commands::api::deep_link_take,
            commands::api::deep_link_ack,
            commands::api::update_check,
            #[cfg(desktop)]
            commands::api::update_install,
            commands::api::updater_status,
            commands::settings::settings_get,
            commands::settings::settings_patch,
            commands::settings::settings_set_game_dir,
            commands::settings::settings_set_download_dir,
            commands::settings::settings_reset,
            commands::fs::fs_roots,
            commands::fs::fs_list_dirs,
            commands::import::import_dropped,
            commands::import::import_preview,
            commands::import::import_paths,
            commands::import::import_unmanaged,
            commands::import::import_managers,
            commands::import::import_manager_scan,
            commands::import::local_list,
            commands::import::local_get,
            commands::import::local_patch,
            commands::import::local_delete,
            commands::import::local_add_to_sandbox,
            commands::logs::log_read,
            commands::logs::log_clear,
            commands::logs::log_path,
            commands::servers::ping_server,
            commands::servers::query_server,
            commands::servers::query_servers,
            commands::servers::latency_series,
            commands::servers::latency_clear,
            #[cfg(desktop)]
            commands::console::console_probe,
            #[cfg(desktop)]
            commands::console::console_run,
            #[cfg(desktop)]
            commands::console::console_stdin,
            #[cfg(desktop)]
            commands::console::console_stdin_close,
            #[cfg(desktop)]
            commands::console::console_stop,
            #[cfg(desktop)]
            commands::console::console_running,
            commands::sessions::session_running,
            commands::sessions::session_history,
            commands::sessions::session_stop,
            commands::sessions::session_log,
            commands::sessions::playtime_summary,
            commands::sessions::sessions_flush,
            commands::detect::detect_scan,
            commands::detect::detect_scan_cancel,
            commands::detect::detect_scan_running,
            commands::detect::detect_apply_many,
            commands::sandbox::sandbox_install_item,
            commands::sandbox::sandbox_export,
            commands::sandbox::sandbox_import_preview,
            commands::sandbox::sandbox_import,
            commands::config::config_available,
            commands::config::config_list,
            commands::config::config_read,
            commands::config::config_write,
            commands::games::games_platform,
            commands::games::games_list,
            commands::games::games_available,
            commands::games::game_native_target,
            commands::games::game_install,
            commands::games::game_uninstall,
            commands::games::game_set_auto_update,
            commands::games::games_check_updates,
            commands::games::games_auto_update,
            commands::games::game_launch,
            commands::games::games_cache_size,
            commands::games::games_cache_clear,
            commands::play::play_open_web,
            commands::play::play_handoff,
            commands::play::play_connect,
            commands::play::play_close,
            commands::play::play_state,
            commands::plugins::plugin_list,
            commands::plugins::plugin_inspect,
            commands::plugins::plugin_trusted_keys,
            commands::plugins::plugin_trust_key,
            commands::plugins::plugin_untrust_key,
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
            commands::sandbox::sandbox_list,
            commands::sandbox::sandbox_get,
            commands::sandbox::sandbox_create,
            commands::sandbox::sandbox_patch,
            commands::sandbox::sandbox_delete,
            commands::sandbox::sandbox_set_default,
            commands::sandbox::sandbox_add_mod,
            commands::sandbox::sandbox_remove_mod,
            commands::sandbox::sandbox_set_mod_enabled,
            commands::sandbox::sandbox_reorder,
            commands::sandbox::sandbox_stage,
            commands::sandbox::sandbox_deploy,
            commands::sandbox::sandbox_purge,
            commands::sandbox::sandbox_verify,
            commands::sandbox::sandbox_strategies,
            commands::sandbox::sandbox_launch_preview,
            commands::sandbox::sandbox_launch,
            commands::sandbox::sandbox_spec,
            commands::sandbox::sandbox_check,
            commands::sandbox::sandbox_refresh_dependencies,
            commands::sandbox::sandbox_add_missing,
            commands::sandbox::sandbox_updates,
            commands::sandbox::sandbox_auto_update,
            commands::downloads::download_list,
            commands::downloads::download_pause,
            commands::downloads::download_resume,
            commands::downloads::download_cancel,
            commands::downloads::download_set_priority,
            commands::downloads::download_set_limit,
            commands::downloads::download_set_global_limit,
            commands::downloads::download_set_concurrency,
            commands::downloads::download_clear_finished,
            commands::downloads::download_release,
            commands::detect::detect_games,
            commands::detect::detect_apply,
            commands::rcon::rcon_list,
            commands::rcon::rcon_create,
            commands::rcon::rcon_update,
            commands::rcon::rcon_set_password,
            commands::rcon::rcon_delete,
            commands::rcon::rcon_connect,
            commands::rcon::rcon_disconnect,
            commands::rcon::rcon_is_connected,
            commands::rcon::rcon_exec,
            commands::rcon::rcon_history,
            commands::rcon::rcon_clear_history,
            commands::rcon::rcon_suggest_protocol,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
