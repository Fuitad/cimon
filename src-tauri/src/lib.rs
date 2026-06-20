mod commands;
mod config;
mod i18n;
mod model;
mod notify;
mod poller;
mod provider;
mod secrets;
mod tray;
mod window;

use tauri::Manager;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

// Embeds the YAML catalogs under src-tauri/locales/ at compile time and generates the `t!`
// macro. Falls back to English for unknown locales or missing keys.
rust_i18n::i18n!("locales", fallback = "en");

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            // Load config and APPLY the locale (inside bootstrap) BEFORE the tray is built and
            // the poller spawns, so the first notifications and the initial tray menu are
            // already localized even when the app starts hidden in the tray.
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("failed to resolve app config dir");
            app.manage(commands::AppState::bootstrap(config_dir));

            // Reconcile OS autostart with the persisted preference.
            {
                let want = app
                    .state::<commands::AppState>()
                    .config
                    .lock()
                    .unwrap()
                    .launch_at_login;
                let autostart = app.autolaunch();
                let is_on = autostart.is_enabled().unwrap_or(false);
                if want && !is_on {
                    let _ = autostart.enable();
                } else if !want && is_on {
                    let _ = autostart.disable();
                }
            }

            // Build the tray (reads the applied locale + monitored set).
            let tray = tray::build_tray(app.handle())?;

            // First-run UX: show the settings window when there are no accounts yet, otherwise
            // start hidden as a quiet menu-bar app. Window visibility drives the macOS dock icon
            // (see `window`): shown -> dock icon visible, hidden -> dock icon hidden.
            let has_accounts = !app
                .state::<commands::AppState>()
                .config
                .lock()
                .unwrap()
                .accounts
                .is_empty();
            if has_accounts {
                window::hide_main(app.handle());
                // First hidden launch only: tell the user we live in the menu bar (which macOS
                // can itself hide when the bar is full), then remember we have shown the notice.
                let state = app.state::<commands::AppState>();
                let notice_locale = {
                    let mut cfg = state.config.lock().unwrap();
                    if cfg.menu_bar_notice_shown {
                        None
                    } else {
                        cfg.menu_bar_notice_shown = true;
                        let _ = config::save(&state.config_path, &cfg);
                        Some(i18n::resolve(&cfg))
                    }
                };
                if let Some(locale) = notice_locale {
                    notify::notify_running_in_menu_bar(app.handle(), &locale);
                }
            } else {
                window::show_main(app.handle());
            }

            // Closing the window hides it (and the dock icon); the app keeps polling.
            if let Some(window) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        window::hide_main(&app_handle);
                    }
                });
            }

            // Spawn the background poller. It shares the same Config the commands mutate and
            // forwards transitions to notifications + the aggregate status to the tray.
            let config = app.state::<commands::AppState>().config.clone();
            let http = provider::build_http_client();
            // Share the command layer's token store (an in-memory cache over the keychain) so the
            // keychain is read at most once per account per run, not once per poll tick.
            let tokens = app.state::<commands::AppState>().tokens.clone();
            let app_for_notify = app.handle().clone();
            let app_for_tray = app.handle().clone();
            let tray_for_status = tray.clone();
            tauri::async_runtime::spawn(async move {
                poller::run_poller(
                    http,
                    tokens,
                    config,
                    move |transitions| {
                        let state = app_for_notify.state::<commands::AppState>();
                        let (rules, locale) = {
                            let cfg = state.config.lock().unwrap();
                            (cfg.rules, i18n::resolve(&cfg))
                        };
                        for tr in transitions {
                            notify::notify_transition(
                                &app_for_notify,
                                tr,
                                &tr.project_name,
                                &rules,
                                &locale,
                            );
                        }
                    },
                    move |status| {
                        tray::set_status(&tray_for_status, status);
                        let _ = tray::refresh_menu(&app_for_tray, &tray_for_status);
                    },
                )
                .await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_account,
            commands::remove_account,
            commands::list_accounts,
            commands::list_discovered_projects,
            commands::get_config,
            commands::get_monitored_projects,
            commands::set_monitored_projects,
            commands::set_notification_rules,
            commands::set_poll_interval,
            commands::set_locale,
            commands::set_launch_at_login,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
