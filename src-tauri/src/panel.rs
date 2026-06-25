//! The tray popover panel: a borderless, always-on-top webview window anchored to the tray icon.
//!
//! Left-clicking the tray toggles this panel (see `tray.rs`); it lists the monitored projects with
//! live CI status and offers Open Settings / Quit. It is created once at startup (hidden) and
//! shown/hidden on demand so the webview stays warm and opening is instant. Positioning is done in
//! Rust: we cache the tray-icon rect from the tray event stream and center the panel under it.
//!
//! Cross-platform: no `#[cfg(target_os)]` here. `TrayCenter` anchors under the icon on macOS (menu
//! bar at the top) and above it on Windows (tray at the bottom); transparency (for the rounded
//! card's corners) is enabled via `app.macOSPrivateApi` in `tauri.conf.json`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::tray::TrayIconEvent;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

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

/// The tray icon's last-known physical rect (position + size), captured from the tray event
/// stream by [`on_tray_event`]. The panel anchors itself under the icon from this, so we never
/// call into a window monitor lookup. Owning this (instead of using tauri-plugin-positioner)
/// avoids that plugin's `current_monitor().unwrap()`, which aborts the whole app on a
/// multi-monitor setup when the panel window's frame is not on any monitor.
static TRAY_RECT: Mutex<Option<(PhysicalPosition<f64>, PhysicalSize<f64>)>> = Mutex::new(None);

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

/// Cache the tray icon's physical rect from the tray event stream so the panel can anchor itself
/// under the icon. Called from the tray's event handler for every tray event. tray-icon reports a
/// physical rect, so the scale factor passed to `to_physical` is irrelevant.
pub fn on_tray_event(event: &TrayIconEvent) {
    let TrayIconEvent::Click { rect, .. } = event else {
        return;
    };
    let position: PhysicalPosition<f64> = rect.position.to_physical(1.0);
    let size: PhysicalSize<f64> = rect.size.to_physical(1.0);
    *TRAY_RECT.lock().unwrap() = Some((position, size));
}

/// Top-left physical position that centers a panel of `panel_size` horizontally under the tray
/// icon described by `tray_pos`/`tray_size`. On macOS the menu bar sits at the top, so a panel
/// placed a full window-height above the icon would land off-screen (negative y); there it drops
/// to just below the icon instead. On Windows the tray sits at the bottom, so the fallback is
/// below the icon (`tray_y + tray_height`). Mirrors tauri-plugin-positioner's TrayCenter math,
/// minus the monitor lookup that made it crash.
fn tray_center_position(
    tray_pos: PhysicalPosition<f64>,
    tray_size: PhysicalSize<f64>,
    panel_size: PhysicalSize<u32>,
) -> PhysicalPosition<i32> {
    let tray_x = tray_pos.x as i32;
    let tray_y = tray_pos.y as i32;
    let tray_width = tray_size.width as i32;
    let _tray_height = tray_size.height as i32;
    let win_width = panel_size.width as i32;
    let win_height = panel_size.height as i32;

    let x = tray_x + tray_width / 2 - win_width / 2;
    let y = tray_y - win_height;
    #[cfg(target_os = "macos")]
    let y = if y < 0 { tray_y } else { y };
    #[cfg(target_os = "windows")]
    let y = if y < 0 { tray_y + _tray_height } else { y };
    PhysicalPosition::new(x, y)
}

/// Anchor the panel centered under the tray icon, using the rect cached by [`on_tray_event`].
/// No-op until a tray event has been seen (the panel only opens from a tray click, which caches
/// the rect first) or if the window size can't be read.
fn anchor_to_tray(win: &WebviewWindow) {
    let Some((tray_pos, tray_size)) = *TRAY_RECT.lock().unwrap() else {
        return;
    };
    let Ok(panel_size) = win.outer_size() else {
        return;
    };
    let _ = win.set_position(tray_center_position(tray_pos, tray_size, panel_size));
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
/// work). Anchoring uses the tray-icon rect cached by the tray event handler.
pub fn show(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(PANEL) {
        anchor_to_tray(&win);
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
            anchor_to_tray(&win);
        }
    }
}

/// Tell an open panel that the status snapshot changed so it re-fetches. Cheap no-op when closed.
pub fn notify_changed(app: &AppHandle) {
    let _ = app.emit(EVENT_STATUS_UPDATED, ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_center_centers_panel_horizontally_under_the_icon() {
        // Tray icon near the top of a right-hand monitor (physical coords), 44x24 px.
        let tray_pos = PhysicalPosition::new(2000.0, 12.0);
        let tray_size = PhysicalSize::new(44.0, 24.0);
        // The panel is 320x360 logical, i.e. 640x720 physical on a 2x (Retina) display.
        let panel_size = PhysicalSize::new(640u32, 720u32);

        let p = tray_center_position(tray_pos, tray_size, panel_size);

        // Horizontally centered under the icon, on every platform.
        assert_eq!(p.x, 2000 + 44 / 2 - 640 / 2);
        // On macOS the menu bar is at the top, so a panel taller than the icon's y would land
        // off the top of the screen; it drops to just below the icon (y == tray_y) instead.
        #[cfg(target_os = "macos")]
        assert_eq!(p.y, 12);
    }
}
