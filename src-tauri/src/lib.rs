mod commands;
mod config;
mod expiry;
mod fixtures;
mod i18n;
mod model;
mod notify;
mod panel;
mod poller;
mod provider;
mod secrets;
mod tray;
mod updates;
mod window;

use tauri::Manager;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

// Embeds the YAML catalogs under src-tauri/locales/ at compile time and generates the `t!`
// macro. Falls back to English for unknown locales or missing keys.
rust_i18n::i18n!("locales", fallback = "en");

/// Warn with a native error dialog when the OS credential store is unreachable at startup.
///
/// Only meaningful on Linux, where the Secret Service (GNOME Keyring / KWallet) may be absent;
/// macOS Keychain and Windows Credential Manager are part of the OS. Skipped for dev / `dev-tokens`
/// builds, which read tokens from a plaintext file rather than the keychain. The gates are runtime
/// `cfg!` checks (not `#[cfg]`) so the body is typechecked on every platform, while only a Linux
/// release build actually probes and warns. The probe is a blocking keychain read, so it runs on a
/// background thread and the dialog is dispatched non-blocking; neither delays startup.
fn warn_if_credential_store_unavailable(app: &tauri::AppHandle, locale: String) {
    if !cfg!(target_os = "linux") || cfg!(any(debug_assertions, feature = "dev-tokens")) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if secrets::KeyringStore::new().probe().is_err() {
            use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
            let (title, body) = notify::format_keychain_unavailable(&locale);
            app.dialog()
                .message(body)
                .kind(MessageDialogKind::Error)
                .title(title)
                .show(|_| {});
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Load config and APPLY the locale (inside bootstrap) BEFORE the tray is built and
            // the poller spawns, so the first notifications and the initial tray menu are
            // already localized even when the app starts hidden in the tray.
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("failed to resolve app config dir");
            app.manage(commands::AppState::bootstrap(config_dir));

            // Dev-only fixtures mode (see fixtures.rs): when active, the state is seeded with fake
            // data and we skip the live poller plus the autostart/notice side effects, then
            // foreground one surface (popover or settings) so it can be screenshotted.
            let fixtures_active = app.state::<commands::AppState>().fixtures.is_some();

            // Pin native notifications to CIMon's identity before the first one can fire (the
            // menu-bar notice below, or a poller transition). Required on macOS, no-op elsewhere.
            notify::init(app.handle());

            // Warn up front (Linux) if the OS credential store is unreachable, so a missing Secret
            // Service is loud at launch instead of surfacing only as silently idle monitoring.
            {
                let state = app.state::<commands::AppState>();
                let locale = {
                    let cfg = state.config.lock().unwrap();
                    i18n::resolve(&cfg)
                };
                warn_if_credential_store_unavailable(app.handle(), locale);
            }

            // Reconcile OS autostart with the persisted preference. Skipped in fixtures mode so a
            // screenshot run never flips the developer's real login-items entry.
            if !fixtures_active {
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

            // Apply the persisted UI mode to the settings window's native chrome before it is
            // shown, so a forced light/dark theme is in place from the first paint.
            {
                let ui_mode = app
                    .state::<commands::AppState>()
                    .config
                    .lock()
                    .unwrap()
                    .ui_mode;
                window::apply_theme(app.handle(), ui_mode);
            }

            // Build the tray (reads the applied locale + monitored set).
            let tray = tray::build_tray(app.handle())?;

            // Build the popover panel (hidden) that left-clicking the tray opens. Created up front
            // so the webview is warm and opening is instant; it dismisses on blur (clicking away).
            let panel_win = panel::build_panel(app.handle())?;
            {
                let app_handle = app.handle().clone();
                panel_win.on_window_event(move |event| match event {
                    // Clicking outside the panel blurs it -> hide (the menu-bar dismiss behavior).
                    // In fixtures mode the panel must stay open while `screencapture` runs (which
                    // steals focus), so blur-to-dismiss is suppressed there.
                    tauri::WindowEvent::Focused(false) if !fixtures_active => {
                        panel::hide(&app_handle)
                    }
                    // It is borderless with no close button, but guard Cmd/Ctrl+W: hide, don't close.
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        panel::hide(&app_handle);
                    }
                    _ => {}
                });
            }

            // First-run UX: show the settings window when there are no accounts yet, otherwise
            // start hidden as a quiet menu-bar app. Window visibility drives the macOS dock icon
            // (see `window`): shown -> dock icon visible, hidden -> dock icon hidden.
            if fixtures_active {
                // Foreground the requested surface: the settings window for the settings shot, or
                // hide it (popover-only) for the menu-bar hero shot. The popover itself is opened
                // below, after the poller block.
                match fixtures::surface() {
                    fixtures::Surface::Settings => window::show_main(app.handle()),
                    fixtures::Surface::Panel => window::hide_main(app.handle()),
                }
            } else {
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
            // forwards transitions to notifications + the aggregate status to the tray. Skipped in
            // fixtures mode (the else branch seeds the tray + popover from fake data instead).
            if !fixtures_active {
                // Restore a dismissal made in a previous run before the first check, so a dismissed
                // update stays hidden and does not re-notify across restarts.
                {
                    let state = app.state::<commands::AppState>();
                    let dismissed = state
                        .config
                        .lock()
                        .unwrap()
                        .dismissed_update_version
                        .clone();
                    state.updates.seed_dismissed_version(dismissed);
                }
                updates::spawn_update_checks(app.handle().clone());

                let config = app.state::<commands::AppState>().config.clone();
                let http = provider::build_http_client();
                // Share the command layer's token store (an in-memory cache over the keychain) so the
                // keychain is read at most once per account per run, not once per poll tick.
                let tokens = app.state::<commands::AppState>().tokens.clone();
                let app_for_notify = app.handle().clone();
                let app_for_tray = app.handle().clone();
                let app_for_tokens = app.handle().clone();
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
                        move |status, snapshot, token_health| {
                            // Publish the per-project AND per-account snapshots to shared state in a
                            // scoped lock (released before anything reads them). The panel reads
                            // project_status (rows) + token_health (per-row auth-failed flag) via
                            // get_project_statuses; the settings UI reads token_health via
                            // get_token_health; the tray glyph reflects the aggregate.
                            {
                                let state = app_for_tray.state::<commands::AppState>();
                                *state.project_status.lock().unwrap() = snapshot.clone();
                                *state.token_health.lock().unwrap() = token_health.clone();
                            }
                            tray::set_status(&tray_for_status, status);
                            // Nudge an open panel to re-fetch the fresh snapshot (cheap when closed).
                            panel::notify_changed(&app_for_tray);
                        },
                        // Token-health events: fire native auth-failed / expiry notifications, localized
                        // in the active locale (the background poller runs without a webview).
                        move |token_events: &[poller::TokenEvent]| {
                            let state = app_for_tokens.state::<commands::AppState>();
                            let locale = {
                                let cfg = state.config.lock().unwrap();
                                i18n::resolve(&cfg)
                            };
                            for ev in token_events {
                                notify::notify_token_event(&app_for_tokens, ev, &locale);
                            }
                        },
                    )
                    .await;
                });
            } else {
                // Fixtures mode: no live poller. Set the tray glyph once to the fixture aggregate,
                // and (popover surface only) open the anchored panel after the webview settles so it
                // can be captured.
                let aggregate = app
                    .state::<commands::AppState>()
                    .fixtures
                    .as_ref()
                    .map(|f| f.aggregate)
                    .unwrap_or(None);
                tray::set_status(&tray, aggregate);
                if fixtures::surface() == fixtures::Surface::Panel {
                    let app_for_panel = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                        panel::show_for_fixtures(&app_for_panel);
                    });
                }
            }

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
            commands::set_ui_mode,
            commands::set_launch_at_login,
            commands::get_project_statuses,
            commands::get_token_health,
            commands::update_account_token,
            commands::open_project_url,
            commands::app_info,
            commands::show_settings_window,
            commands::quit_app,
            commands::hide_panel,
            commands::set_panel_height,
            commands::get_update_state,
            commands::check_for_updates,
            commands::install_update,
            commands::dismiss_update,
            commands::open_update_release,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
