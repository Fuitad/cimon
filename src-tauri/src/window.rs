//! Settings-window visibility and its macOS dock-icon linkage.
//!
//! CIMon is a menu-bar app: while the settings window is closed it should live only in the menu
//! bar, with no dock icon. Showing the window reveals the dock icon; hiding it removes the dock
//! icon again. The two are kept in lockstep here so every entry point (startup, the window's
//! close button, the tray's Open Settings) goes through the same path and cannot drift.
//!
//! The dock-icon concept is macOS-only (`NSApplicationActivationPolicy`); on Windows/Linux the
//! activation-policy calls are compiled out and only the window show/hide remains.

use tauri::{AppHandle, Manager};

/// Label of the single settings window (see `tauri.conf.json`).
pub const MAIN: &str = "main";

/// Show the settings window, focus it, and (macOS) reveal the dock icon.
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        set_dock_visible(app, true);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Hide the settings window and (macOS) hide the dock icon, leaving a pure menu-bar app.
pub fn hide_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.hide();
        set_dock_visible(app, false);
    }
}

/// Link dock-icon visibility to window visibility. macOS only; a no-op elsewhere.
#[cfg(target_os = "macos")]
fn set_dock_visible(app: &AppHandle, visible: bool) {
    let policy = if visible {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    };
    let _ = app.set_activation_policy(policy);
}

#[cfg(not(target_os = "macos"))]
fn set_dock_visible(_app: &AppHandle, _visible: bool) {}
