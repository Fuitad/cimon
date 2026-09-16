//! The tray popover panel: a borderless, always-on-top webview window anchored to the tray icon.
//!
//! Left-clicking the tray toggles this panel (see `tray.rs`); it lists the monitored projects with
//! live CI status and offers Open Settings / Quit. It is created once at startup (hidden) and
//! shown/hidden on demand so the webview stays warm and opening is instant. Positioning is done in
//! Rust: we cache the tray-icon rect from the tray event stream and center the panel under it.
//!
//! Cross-platform: the panel anchors under the icon on macOS (menu bar at the top, positioned in
//! logical points to survive mixed-scale displays) and above it on Windows (tray at the bottom);
//! transparency (for the rounded card's corners) is enabled via `app.macOSPrivateApi` in
//! `tauri.conf.json`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::tray::TrayIconEvent;
#[cfg(target_os = "macos")]
use tauri::LogicalPosition;
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

/// macOS: the scale factor of the display the tray icon was last clicked on, captured alongside
/// [`TRAY_RECT`] because the rect's pixels are in that display's scale and the rect alone cannot
/// tell which one it is (see [`macos_panel_position`]). Captured at click time so a later re-anchor
/// (a content height change) does not depend on where the cursor has moved since.
#[cfg(target_os = "macos")]
static TRAY_SCALE: Mutex<Option<f64>> = Mutex::new(None);

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
/// physical rect, so the scale factor passed to `to_physical` is irrelevant. On macOS it also
/// records the scale of the display under the cursor, i.e. the one whose menu bar was clicked.
pub fn on_tray_event(app: &AppHandle, event: &TrayIconEvent) {
    let TrayIconEvent::Click { rect, .. } = event else {
        return;
    };
    let position: PhysicalPosition<f64> = rect.position.to_physical(1.0);
    let size: PhysicalSize<f64> = rect.size.to_physical(1.0);
    *TRAY_RECT.lock().unwrap() = Some((position, size));
    #[cfg(target_os = "macos")]
    {
        *TRAY_SCALE.lock().unwrap() = cursor_display_scale(app);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// Scale factor of the display under the cursor (macOS). tao reports the cursor in the primary
/// display's scale, so dividing by it gives global logical points, the one coordinate space in which
/// displays never overlap.
#[cfg(target_os = "macos")]
fn cursor_display_scale(app: &AppHandle) -> Option<f64> {
    let cursor = app.cursor_position().ok()?;
    let primary_scale = app.primary_monitor().ok()??.scale_factor();
    let monitors: Vec<_> = app
        .available_monitors()
        .ok()?
        .iter()
        .map(|m| (*m.position(), *m.size(), m.scale_factor()))
        .collect();
    monitor_scale_at(
        LogicalPosition::new(cursor.x / primary_scale, cursor.y / primary_scale),
        &monitors,
    )
}

/// Scale factor of the monitor whose global logical bounds contain `point`. tao reports each
/// monitor's origin and size in that monitor's own pixels, so each is divided by its own scale.
#[cfg(target_os = "macos")]
fn monitor_scale_at(
    point: LogicalPosition<f64>,
    monitors: &[(PhysicalPosition<i32>, PhysicalSize<u32>, f64)],
) -> Option<f64> {
    monitors
        .iter()
        .find(|(pos, size, s)| {
            let (left, top) = (pos.x as f64 / s, pos.y as f64 / s);
            let (width, height) = (size.width as f64 / s, size.height as f64 / s);
            point.x >= left && point.x < left + width && point.y >= top && point.y < top + height
        })
        .map(|(_, _, s)| *s)
}

/// Top-left physical position that centers a panel of `panel_size` horizontally under the tray
/// icon described by `tray_pos`/`tray_size`. The Windows tray sits at the bottom, so the panel goes
/// above the icon, falling back to below it (`tray_y + tray_height`) when that would land
/// off-screen. Mirrors tauri-plugin-positioner's TrayCenter math, minus the monitor lookup that made
/// it crash. macOS uses [`macos_panel_position`] instead.
#[cfg(not(target_os = "macos"))]
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
    #[cfg(target_os = "windows")]
    let y = if y < 0 { tray_y + _tray_height } else { y };
    PhysicalPosition::new(x, y)
}

/// Top-left position, in global logical points, that centers a `panel_width`-wide panel under the
/// tray icon on macOS, with its top at the top of the icon's menu bar (AppKit then keeps the window
/// just below the bar).
///
/// macOS has no global physical coordinate space: tray-icon reports the rect in the pixels of the
/// display the status item sits on, while `set_position` with a physical position divides by the
/// PANEL window's scale. With the panel on a 2x built-in display and the icon on a 1x external
/// monitor, that halves the coordinates and drops the panel on the wrong display. So the rect is
/// converted to points with `scale`, the scale of the display that was clicked. That scale cannot be
/// inferred from the rect: with a 1x external top-aligned beside a 2x built-in, the external icon's
/// 1x pixels also fall inside the built-in's 2x pixel rectangle. [`on_tray_event`] resolves it from
/// the cursor instead.
#[cfg(target_os = "macos")]
fn macos_panel_position(
    tray_pos: PhysicalPosition<f64>,
    tray_size: PhysicalSize<f64>,
    panel_width: f64,
    scale: f64,
) -> LogicalPosition<f64> {
    let center_x = tray_pos.x + tray_size.width / 2.0;
    LogicalPosition::new(center_x / scale - panel_width / 2.0, tray_pos.y / scale)
}

/// Anchor the panel centered under the tray icon, using the rect cached by [`on_tray_event`].
/// No-op until a tray event has been seen (the panel only opens from a tray click, which caches
/// the rect first) or if the window size can't be read.
fn anchor_to_tray(win: &WebviewWindow) {
    let Some((tray_pos, tray_size)) = *TRAY_RECT.lock().unwrap() else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        let scale = match *TRAY_SCALE.lock().unwrap() {
            Some(scale) => scale,
            None => match win.scale_factor() {
                Ok(scale) => scale,
                Err(_) => return,
            },
        };
        let _ = win.set_position(macos_panel_position(
            tray_pos,
            tray_size,
            PANEL_WIDTH,
            scale,
        ));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let Ok(panel_size) = win.outer_size() else {
            return;
        };
        let _ = win.set_position(tray_center_position(tray_pos, tray_size, panel_size));
    }
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

/// Dev-only (fixtures mode): open the popover as if the tray icon sat at the top-right of the
/// primary monitor, so it can be screenshotted without a physical menu-bar click (which an agent
/// cannot synthesize). The synthetic anchor sits in the menu-bar extras zone, so the popover hangs
/// just under the bar; nudge it horizontally with `CIMON_FIXTURES_TRAY_X` (logical px in from the
/// right edge) to line it up under the real CIMon glyph.
pub fn show_for_fixtures(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(PANEL) {
        if let Ok(Some(monitor)) = win.primary_monitor() {
            let size = monitor.size(); // physical pixels
            let scale = monitor.scale_factor();
            let inset = std::env::var("CIMON_FIXTURES_TRAY_X")
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(210.0)
                * scale;
            let pos = PhysicalPosition::new(size.width as f64 - inset, 2.0);
            *TRAY_RECT.lock().unwrap() = Some((pos, PhysicalSize::new(40.0 * scale, 24.0 * scale)));
            #[cfg(target_os = "macos")]
            {
                *TRAY_SCALE.lock().unwrap() = Some(scale);
            }
        }
    }
    show(app);
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn tray_center_centers_panel_horizontally_under_the_icon() {
        // Tray icon near the top of a right-hand monitor (physical coords), 44x24 px.
        let tray_pos = PhysicalPosition::new(2000.0, 12.0);
        let tray_size = PhysicalSize::new(44.0, 24.0);
        // The panel is 320x360 logical, i.e. 640x720 physical on a 2x (Retina) display.
        let panel_size = PhysicalSize::new(640u32, 720u32);

        let p = tray_center_position(tray_pos, tray_size, panel_size);

        // Horizontally centered under the icon.
        assert_eq!(p.x, 2000 + 44 / 2 - 640 / 2);
    }

    /// Where the panel goes on macOS for a click with the cursor at `cursor` (global logical
    /// points) on an icon whose rect tray-icon reported as `tray_pos`/`tray_size`.
    #[cfg(target_os = "macos")]
    fn place(
        monitors: &[(PhysicalPosition<i32>, PhysicalSize<u32>, f64)],
        cursor: (f64, f64),
        tray_pos: (f64, f64),
        tray_size: (f64, f64),
    ) -> LogicalPosition<f64> {
        let scale = monitor_scale_at(LogicalPosition::new(cursor.0, cursor.1), monitors)
            .expect("cursor is on a monitor");
        macos_panel_position(
            PhysicalPosition::new(tray_pos.0, tray_pos.1),
            PhysicalSize::new(tray_size.0, tray_size.1),
            PANEL_WIDTH,
            scale,
        )
    }

    /// A 2x built-in display at the origin and a 1x external immediately to its right, with the
    /// external's top at `external_top`. tao reports each monitor's origin and size in that monitor's
    /// own pixels.
    #[cfg(target_os = "macos")]
    fn built_in_and_external(
        external_top: i32,
    ) -> [(PhysicalPosition<i32>, PhysicalSize<u32>, f64); 2] {
        [
            (
                PhysicalPosition::new(0, 0),
                PhysicalSize::new(3456, 2234),
                2.0,
            ),
            (
                PhysicalPosition::new(1728, external_top),
                PhysicalSize::new(1920, 1080),
                1.0,
            ),
        ]
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_panel_lands_under_the_icon_on_the_clicked_display_with_mixed_scales() {
        let external_expected = LogicalPosition::new(2763.0 + 17.0 - PANEL_WIDTH / 2.0, -574.0);
        let built_in_expected = LogicalPosition::new(570.0 + 17.0 - PANEL_WIDTH / 2.0, 0.0);

        // External raised above the built-in. The icon on the external is reported at 1x, the one
        // on the built-in at 2x, while the hidden panel window sits on the built-in.
        let monitors = built_in_and_external(-572);
        let external_click = ((2780.0, -557.0), (2763.0, -574.0), (34.0, 33.0));
        let built_in_click = ((587.0, 16.0), (1140.0, 0.0), (68.0, 66.0));
        let (c, t, s) = external_click;
        assert_eq!(place(&monitors, c, t, s), external_expected);
        let (c, t, s) = built_in_click;
        assert_eq!(place(&monitors, c, t, s), built_in_expected);

        // Tops aligned: the external icon's 1x pixels also fall inside the built-in's 2x pixel
        // rectangle, so only the cursor's logical position can tell the displays apart. The result
        // must not depend on the order the monitors are listed in.
        let mut monitors = built_in_and_external(0);
        for _ in 0..2 {
            let p = place(&monitors, (2780.0, 16.0), (2763.0, 0.0), (34.0, 33.0));
            assert_eq!(p, LogicalPosition::new(external_expected.x, 0.0));
            let (c, t, s) = built_in_click;
            assert_eq!(place(&monitors, c, t, s), built_in_expected);
            monitors.reverse();
        }
    }
}
