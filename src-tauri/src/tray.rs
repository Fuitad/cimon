//! System tray / menu-bar presence: an aggregate-status icon and a menu listing monitored
//! projects, with Open Settings and Quit. Labels are localized via `rust-i18n`; the tray reads
//! the GLOBAL locale, so callers MUST `i18n::apply`/`set_locale` before building or rebuilding
//! it (Tasks 5/11). Clicking a project opens its pipeline page in the browser.

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_opener::OpenerExt;

use crate::commands::AppState;
use crate::model::PipelineStatus;

const OPEN_PREFIX: &str = "open|";
const SETTINGS_ID: &str = "cimon-settings";
const QUIT_ID: &str = "cimon-quit";
const TRAY_ID: &str = "cimon-tray";

/// RGBA color for the aggregate tray icon. `None` = idle (nothing tracked); idle is white and
/// drawn as a macOS template (see `set_status`) so the menu bar keeps it visible on any
/// background. Placeholder palette for Milestone 1; final iconography is the `impeccable` pass.
pub fn status_color(status: Option<PipelineStatus>) -> [u8; 4] {
    match status {
        Some(PipelineStatus::Failed) => [0xD4, 0x33, 0x33, 0xFF], // red
        Some(PipelineStatus::Running) => [0x33, 0x77, 0xD4, 0xFF], // blue
        Some(PipelineStatus::Pending) | Some(PipelineStatus::Manual) => [0xD4, 0xA3, 0x33, 0xFF], // amber
        Some(_) => [0x33, 0xA8, 0x53, 0xFF], // green (settled/success)
        None => [0xFF, 0xFF, 0xFF, 0xFF],    // white (idle)
    }
}

/// Output size of the rendered glyph (square). The menu bar scales it to ~18pt tall, so this is
/// chosen for crisp downscaling on Retina/2-3x displays.
const ICON_N: u32 = 64;

/// Draw the CIMon logo glyph (outer ring + central orb + a three-dot pipeline motif) filled with
/// `color`, anti-aliased on a transparent background. Geometry mirrors the app icon
/// (`icons/*.png`) in its 256-unit design space. Active states are colored glyphs that carry the
/// aggregate status by tint; the idle state is white and flagged as a macOS template by the
/// caller so the system recolors it for the current menu bar. A Milestone-1 rendering, pending
/// the final iconography pass.
fn logo_icon(color: [u8; 4]) -> tauri::image::Image<'static> {
    const N: u32 = ICON_N;
    // One output pixel expressed in the 256-unit design space; the anti-alias band width.
    let aa = 256.0 / N as f64;

    // Geometry in the 256-unit design space.
    let center = (128.0, 128.0);
    let (ring_outer, ring_inner) = (104.0, 92.0);
    let (orb_c, orb_r) = ((128.0, 116.0), 30.0);
    let pipe_y = 190.0;
    let dot_xs = [78.0, 128.0, 178.0];
    let dot_r = 13.0;
    let connector_half = 4.0;

    // Coverage of one filled disk at point p (1.0 inside, 0.0 outside, AA across the edge).
    let disk = |p: (f64, f64), c: (f64, f64), r: f64| -> f64 {
        let d = ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt();
        ((r - d) / aa + 0.5).clamp(0.0, 1.0)
    };

    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for j in 0..N {
        for i in 0..N {
            let p = (
                (i as f64 + 0.5) * 256.0 / N as f64,
                (j as f64 + 0.5) * 256.0 / N as f64,
            );
            // Ring = inside the outer edge AND outside the inner hole.
            let d_center = ((p.0 - center.0).powi(2) + (p.1 - center.1).powi(2)).sqrt();
            let ring = (((ring_outer - d_center) / aa + 0.5).clamp(0.0, 1.0))
                .min(((d_center - ring_inner) / aa + 0.5).clamp(0.0, 1.0));
            // Orb.
            let orb = disk(p, orb_c, orb_r);
            // Pipeline: horizontal connector segment plus three dots.
            let connector = if p.0 >= dot_xs[0] && p.0 <= dot_xs[2] {
                ((connector_half - (p.1 - pipe_y).abs()) / aa + 0.5).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let dots = dot_xs
                .iter()
                .map(|&x| disk(p, (x, pipe_y), dot_r))
                .fold(0.0_f64, f64::max);

            let coverage = ring.max(orb).max(connector).max(dots);
            rgba.push(color[0]);
            rgba.push(color[1]);
            rgba.push(color[2]);
            rgba.push((coverage * 255.0).round() as u8);
        }
    }
    tauri::image::Image::new_owned(rgba, N, N)
}

/// Build the tray menu from the current config (monitored projects + active locale).
fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<AppState>();
    let (monitored, locale) = {
        let cfg = state.config.lock().unwrap();
        (cfg.monitored.clone(), crate::i18n::resolve(&cfg))
    };

    let mut builder = MenuBuilder::new(app);
    if monitored.is_empty() {
        let none = MenuItemBuilder::with_id(
            "cimon-none",
            &rust_i18n::t!("tray.no_projects", locale = locale),
        )
        .enabled(false)
        .build(app)?;
        builder = builder.item(&none);
    } else {
        for mp in &monitored {
            // Encode the target URL in the item id so the click handler can open it.
            let item = MenuItemBuilder::with_id(format!("{OPEN_PREFIX}{}", mp.web_url), &mp.name)
                .build(app)?;
            builder = builder.item(&item);
        }
    }

    let settings = MenuItemBuilder::with_id(
        SETTINGS_ID,
        &rust_i18n::t!("tray.open_settings", locale = locale),
    )
    .build(app)?;
    let quit = MenuItemBuilder::with_id(QUIT_ID, &rust_i18n::t!("tray.quit", locale = locale))
        .build(app)?;

    builder.separator().item(&settings).item(&quit).build()
}

/// Create the tray icon with its menu and handlers. Call once during setup (Task 11).
pub fn build_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let menu = build_menu(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(logo_icon(status_color(None)))
        // Starts idle: render the white glyph as a template so macOS keeps it visible (white on a
        // dark menu bar, dark on a light one) rather than a fixed colour that can vanish.
        .icon_as_template(true)
        .menu(&menu)
        .on_menu_event(|app: &AppHandle, event: tauri::menu::MenuEvent| {
            let id = event.id().as_ref();
            if id == QUIT_ID {
                app.exit(0);
            } else if id == SETTINGS_ID {
                show_settings(app);
            } else if let Some(url) = id.strip_prefix(OPEN_PREFIX) {
                let _ = app.opener().open_url(url.to_string(), None::<&str>);
            }
        })
        .build(app)
}

/// Update the tray icon to reflect the aggregate worst status. Call from the poller (Task 11).
pub fn set_status(tray: &TrayIcon, status: Option<PipelineStatus>) {
    let _ = tray.set_icon(Some(logo_icon(status_color(status))));
    // Idle has no status colour to convey, so render it as a template image: macOS draws it in
    // the menu bar's own colour (white on a dark bar, dark on a light one) so it stays visible.
    // Active states keep their colour to convey status.
    let _ = tray.set_icon_as_template(status.is_none());
}

/// Rebuild the tray menu after the monitored set or locale changes (Tasks 5/11).
pub fn refresh_menu(app: &AppHandle, tray: &TrayIcon) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    tray.set_menu(Some(menu))
}

/// Look up the live tray by id and rebuild its menu now. Called from the commands that change
/// the monitored set or the locale so the tray updates immediately rather than on the next poll.
pub fn refresh(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = refresh_menu(app, &tray);
    }
}

fn show_settings(app: &AppHandle) {
    // Show + focus the window and reveal the dock icon (window visibility drives dock visibility).
    crate::window::show_main(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_icon_is_tinted_glyph_on_transparent_background() {
        let red = status_color(Some(PipelineStatus::Failed));
        let img = logo_icon(red);
        assert_eq!(img.width(), ICON_N);
        assert_eq!(img.height(), ICON_N);
        let rgba = img.rgba();
        let px = |x: u32, y: u32| {
            let i = ((y * ICON_N + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        };
        // Center sits inside the orb: opaque and carrying the status color.
        let c = px(ICON_N / 2, (116 * ICON_N) / 256);
        assert_eq!([c[0], c[1], c[2]], [red[0], red[1], red[2]]);
        assert!(
            c[3] > 200,
            "glyph center should be (near) opaque, got {c:?}"
        );
        // A corner is outside the glyph: fully transparent.
        assert_eq!(px(0, 0)[3], 0, "corner should be transparent");
    }

    #[test]
    fn status_color_distinguishes_key_states() {
        assert_ne!(
            status_color(Some(PipelineStatus::Failed)),
            status_color(Some(PipelineStatus::Success))
        );
        assert_ne!(
            status_color(Some(PipelineStatus::Running)),
            status_color(None)
        );
        // Idle is white (rendered as a template so the menu bar keeps it visible).
        assert_eq!(status_color(None), [0xFF, 0xFF, 0xFF, 0xFF]);
        // Failed is red-dominant.
        let red = status_color(Some(PipelineStatus::Failed));
        assert!(red[0] > red[1] && red[0] > red[2]);
        // Success is green-dominant.
        let green = status_color(Some(PipelineStatus::Success));
        assert!(green[1] > green[0] && green[1] > green[2]);
    }
}
