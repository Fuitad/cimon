//! The tray popover panel: a borderless, always-on-top webview window anchored to the tray icon.
//!
//! Left-clicking the tray toggles this panel (see `tray.rs`); it lists the monitored projects with
//! live CI status and offers Open Settings / Quit. It is created once at startup (hidden) and
//! shown/hidden on demand so the webview stays warm and opening is instant. Positioning is done in
//! Rust via `tauri-plugin-positioner`, which caches the tray-icon rect from the tray event stream.
//!
//! Cross-platform: no `#[cfg(target_os)]` here. `TrayCenter` anchors under the icon on macOS (menu
//! bar at the top) and above it on Windows (tray at the bottom); transparency (for the rounded
//! card's corners) is enabled via `app.macOSPrivateApi` in `tauri.conf.json`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_positioner::{Position, WindowExt};

/// Window label for the panel (its capability in `capabilities/panel.json` is scoped to this).
pub const PANEL: &str = "panel";

/// Event emitted to the panel whenever the per-project status snapshot changes (each poll tick, and
/// on a monitored-set change), so an open panel refreshes live. The panel re-fetches via the
/// `get_project_statuses` command rather than receiving a payload, keeping one serialization path.
const EVENT_STATUS_UPDATED: &str = "status-updated";

/// Fixed panel width; height is driven by content via [`set_height`], clamped to the bounds below.
const PANEL_WIDTH: f64 = 320.0;
const PANEL_INITIAL_HEIGHT: f64 = 360.0;
const PANEL_MIN_HEIGHT: f64 = 96.0;
const PANEL_MAX_HEIGHT: f64 = 540.0;

/// Clicking the tray icon while the panel is open first BLURS the panel (which hides it via the
/// `Focused(false)` handler), and only then delivers the click that would toggle it. Without a
/// guard, that click would immediately reopen the panel the blur just closed. We record when the
/// panel was last hidden and suppress a reopen that lands within this window.
static LAST_HIDDEN: Mutex<Option<Instant>> = Mutex::new(None);
const REOPEN_GUARD: Duration = Duration::from_millis(250);

/// Create the panel window (hidden). Called once during setup, after the tray exists.
pub fn build_panel(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("panel.html".into()))
        .title("CIMon")
        .inner_size(PANEL_WIDTH, PANEL_INITIAL_HEIGHT)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        // We draw the card's shadow in CSS over a transparent window; the OS shadow would trace the
        // rectangular window bounds instead of the rounded card, so it is disabled.
        .shadow(false)
        .build()
}

/// Toggle the panel: hide it if visible, otherwise anchor + show it. Called from the tray's
/// left-click handler.
pub fn toggle(app: &AppHandle) {
    let Some(win) = app.get_webview_window(PANEL) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        hide(app);
        return;
    }
    // Suppress the reopen that the blur-then-click race would otherwise cause (see LAST_HIDDEN).
    if let Some(t) = *LAST_HIDDEN.lock().unwrap() {
        if t.elapsed() < REOPEN_GUARD {
            return;
        }
    }
    show(app);
}

/// Anchor the panel to the tray icon, show it, and focus it (focus is what makes blur-to-dismiss
/// work). The positioner uses the tray-icon rect cached by the tray event handler.
pub fn show(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(PANEL) {
        let _ = win.move_window(Position::TrayCenter);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Hide the panel and stamp the hide time so a tray click that caused the blur does not reopen it.
pub fn hide(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(PANEL) {
        let _ = win.hide();
        *LAST_HIDDEN.lock().unwrap() = Some(Instant::now());
    }
}

/// Resize the panel to fit its content height (clamped), then re-anchor if it is visible. Driven by
/// the panel measuring its own rendered height and calling the `set_panel_height` command, so the
/// popover hugs its content (a few projects) yet caps and scrolls when there are many.
pub fn set_height(app: &AppHandle, height: f64) {
    if let Some(win) = app.get_webview_window(PANEL) {
        let h = height.clamp(PANEL_MIN_HEIGHT, PANEL_MAX_HEIGHT);
        let _ = win.set_size(LogicalSize::new(PANEL_WIDTH, h));
        // Re-anchor only when visible: the tray-rect cache is fresh from the click that showed it,
        // and on Windows (panel sits ABOVE the icon) a height change moves the top edge.
        if win.is_visible().unwrap_or(false) {
            let _ = win.move_window(Position::TrayCenter);
        }
    }
}

/// Tell an open panel that the status snapshot changed so it re-fetches. Cheap no-op when closed.
pub fn notify_changed(app: &AppHandle) {
    let _ = app.emit(EVENT_STATUS_UPDATED, ());
}
