//! System tray / menu-bar presence: an aggregate-status icon and a menu listing monitored
//! projects, with Open Settings and Quit. Labels are localized via `rust-i18n`; the tray reads
//! the GLOBAL locale, so callers MUST `i18n::apply`/`set_locale` before building or rebuilding
//! it (Tasks 5/11). Clicking a project opens its pipeline page in the browser.

use tauri::menu::{IconMenuItemBuilder, Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_opener::OpenerExt;

use crate::commands::AppState;
use crate::model::PipelineStatus;
use crate::poller::{ProjectKey, ProjectStatusView};

const OPEN_PREFIX: &str = "open|";
const SETTINGS_ID: &str = "cimon-settings";
const QUIT_ID: &str = "cimon-quit";
const TRAY_ID: &str = "cimon-tray";

/// Shared RGBA status palette. The active-state colors are used by BOTH the aggregate tray icon
/// ([`status_color`]) and the per-row dots ([`menu_status_color`]); the two differ only in their
/// fallback (idle/settled) handling, so the shared colors live here to avoid drift.
const COLOR_RED: [u8; 4] = [0xD4, 0x33, 0x33, 0xFF]; // failed
const COLOR_BLUE: [u8; 4] = [0x33, 0x77, 0xD4, 0xFF]; // running
const COLOR_AMBER: [u8; 4] = [0xD4, 0xA3, 0x33, 0xFF]; // pending / manual
const COLOR_GREEN: [u8; 4] = [0x33, 0xA8, 0x53, 0xFF]; // success / settled
const COLOR_GREY: [u8; 4] = [0x9A, 0x9A, 0x9A, 0xFF]; // unknown / not-yet-polled (rows only)
const COLOR_WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF]; // idle (aggregate icon only)

/// RGBA color for the aggregate tray icon. `None` = idle (nothing tracked); idle is white and
/// drawn as a macOS template (see `set_status`) so the menu bar keeps it visible on any
/// background. Placeholder palette for Milestone 1; final iconography is the `impeccable` pass.
pub fn status_color(status: Option<PipelineStatus>) -> [u8; 4] {
    match status {
        Some(PipelineStatus::Failed) => COLOR_RED,
        Some(PipelineStatus::Running) => COLOR_BLUE,
        Some(PipelineStatus::Pending) | Some(PipelineStatus::Manual) => COLOR_AMBER,
        Some(_) => COLOR_GREEN, // settled/success
        None => COLOR_WHITE,    // idle
    }
}

/// Output size of the rendered glyph (square). The menu bar scales it to ~18pt tall, so this is
/// chosen for crisp downscaling on Retina/2-3x displays.
const ICON_N: u32 = 64;

/// Anti-aliased coverage (0.0..=1.0) of a filled disc at point `p`, centered at `c` with radius
/// `r`, where `aa` is the width (in the same units as `p`) of the soft edge band. Shared by the
/// logo glyph and the per-row status dots so both anti-alias the disc edge identically.
fn disc_coverage(p: (f64, f64), c: (f64, f64), r: f64, aa: f64) -> f64 {
    let d = ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt();
    ((r - d) / aa + 0.5).clamp(0.0, 1.0)
}

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
    let disk = |p: (f64, f64), c: (f64, f64), r: f64| -> f64 { disc_coverage(p, c, r, aa) };

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

/// Per-row status color for the menu dots. Distinct from [`status_color`] (the aggregate icon
/// palette): per row, settled non-success states and the not-yet-polled state read as a neutral
/// grey rather than green/white, so a canceled run never looks successful and an unknown row is
/// still visible. The status word on the row carries the precise state.
fn menu_status_color(status: Option<PipelineStatus>) -> [u8; 4] {
    match status {
        Some(PipelineStatus::Failed) => COLOR_RED,
        Some(PipelineStatus::Running) => COLOR_BLUE,
        Some(PipelineStatus::Pending) | Some(PipelineStatus::Manual) => COLOR_AMBER,
        Some(PipelineStatus::Success) => COLOR_GREEN,
        // Canceled / Skipped / Other / not-yet-polled: neutral grey (balanced channels).
        _ => COLOR_GREY,
    }
}

/// Render a small filled status disc for a menu row, anti-aliased on a transparent background.
/// A plain disc reads far better than the full logo glyph at menu-icon size, and (unlike the
/// aggregate tray icon) is never a template image, so its color shows on macOS and Windows alike.
fn status_dot(color: [u8; 4]) -> tauri::image::Image<'static> {
    const N: u32 = 36;
    // One output pixel of anti-alias band (design space == pixel space at this size).
    let aa = 1.0_f64;
    let center = (N as f64 / 2.0, N as f64 / 2.0);
    let radius = N as f64 * 0.42;
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for j in 0..N {
        for i in 0..N {
            let p = (i as f64 + 0.5, j as f64 + 0.5);
            let coverage = disc_coverage(p, center, radius, aa);
            rgba.push(color[0]);
            rgba.push(color[1]);
            rgba.push(color[2]);
            rgba.push((coverage * 255.0).round() as u8);
        }
    }
    tauri::image::Image::new_owned(rgba, N, N)
}

/// Build a project row's label: name, latest branch (when known), and the localized status word.
/// A project with no snapshot yet (e.g. before the first poll, or with no current pipeline) is
/// shown as `name (unknown)` so a monitored row is never a bare name. Mirrors the `(status)`
/// style of the notification body, reusing the existing `status.*` catalog (no new keys).
fn row_label(name: &str, view: Option<&ProjectStatusView>, locale: &str) -> String {
    match view {
        Some(v) => {
            let word = rust_i18n::t!(v.status.i18n_key(), locale = locale);
            // When the latest poll failed, keep the last-known status word and append an offline
            // marker, e.g. "cimon  main (succeeded, offline)".
            let detail = if v.stale {
                let offline = rust_i18n::t!("tray.offline", locale = locale);
                format!("{word}, {offline}")
            } else {
                word.to_string()
            };
            if v.branch.is_empty() {
                format!("{name} ({detail})")
            } else {
                let branch = &v.branch;
                format!("{name}  {branch} ({detail})")
            }
        }
        None => {
            let word = rust_i18n::t!(PipelineStatus::Other.i18n_key(), locale = locale);
            format!("{name} ({word})")
        }
    }
}

/// Dot color for a project row: grey when the row is stale (its latest poll failed) regardless of
/// the last-known status, otherwise the per-status color. An unknown (never-polled) row is grey.
fn row_dot_color(view: Option<&ProjectStatusView>) -> [u8; 4] {
    match view {
        Some(v) if v.stale => COLOR_GREY,
        Some(v) => menu_status_color(Some(v.status)),
        None => menu_status_color(None),
    }
}

/// Build the tray menu from the current config (monitored projects + active locale) and the
/// latest per-project status snapshot.
fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<AppState>();
    let (monitored, locale) = {
        let cfg = state.config.lock().unwrap();
        (cfg.monitored.clone(), crate::i18n::resolve(&cfg))
    };
    // Per-project status snapshot from the poller (empty until the first poll completes). Taken
    // as a short, separate lock from `config` above and released before the rows are built.
    let snapshot = state.project_status.lock().unwrap().clone();

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
            let key: ProjectKey = (mp.account_id.clone(), mp.project_id);
            let view = snapshot.get(&key);
            // Encode the target URL in the item id so the click handler can open it; the colored
            // dot + label convey the project's current CI status, branch and status word.
            let item = IconMenuItemBuilder::with_id(
                format!("{OPEN_PREFIX}{}", mp.web_url),
                row_label(&mp.name, view, &locale),
            )
            .icon(status_dot(row_dot_color(view)))
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

    #[test]
    fn row_label_includes_branch_and_status_word_for_known_status() {
        let view = ProjectStatusView {
            status: PipelineStatus::Failed,
            branch: "develop".into(),
            stale: false,
        };
        assert_eq!(
            row_label("backend", Some(&view), "en"),
            "backend  develop (failed)"
        );
        // Empty branch: omit the branch segment, keep the status word.
        let no_branch = ProjectStatusView {
            status: PipelineStatus::Running,
            branch: String::new(),
            stale: false,
        };
        assert_eq!(row_label("api", Some(&no_branch), "en"), "api (running)");
    }

    #[test]
    fn row_label_unknown_project_is_never_bare() {
        // A monitored project with no snapshot yet (startup, before the first poll) must still
        // carry a status word, not revert to a bare name.
        let label = row_label("cimon", None, "en");
        assert_eq!(label, "cimon (unknown)");
        assert_ne!(
            label, "cimon",
            "unknown row must not be a bare project name"
        );
    }

    #[test]
    fn stale_project_row_is_offline_and_grey() {
        let stale_view = ProjectStatusView {
            status: PipelineStatus::Success,
            branch: "main".into(),
            stale: true,
        };
        // The label keeps the last-known status word and appends the offline marker.
        assert_eq!(
            row_label("cimon", Some(&stale_view), "en"),
            "cimon  main (succeeded, offline)"
        );
        // The dot is grey when stale, regardless of the (success) last-known status.
        assert_eq!(row_dot_color(Some(&stale_view)), COLOR_GREY);
        // A fresh success row keeps its green dot and carries no offline marker.
        let fresh = ProjectStatusView {
            status: PipelineStatus::Success,
            branch: "main".into(),
            stale: false,
        };
        assert_eq!(
            row_label("cimon", Some(&fresh), "en"),
            "cimon  main (succeeded)"
        );
        assert_eq!(row_dot_color(Some(&fresh)), COLOR_GREEN);
        // An unknown (never-polled) row is grey too.
        assert_eq!(row_dot_color(None), COLOR_GREY);
    }

    #[test]
    fn menu_status_color_distinguishes_states_and_greys_unknown() {
        let failed = menu_status_color(Some(PipelineStatus::Failed));
        let running = menu_status_color(Some(PipelineStatus::Running));
        let success = menu_status_color(Some(PipelineStatus::Success));
        assert_ne!(failed, running);
        assert_ne!(failed, success);
        assert_ne!(running, success);
        // Failed red-dominant, success green-dominant, running blue-dominant.
        assert!(failed[0] > failed[1] && failed[0] > failed[2]);
        assert!(success[1] > success[0] && success[1] > success[2]);
        assert!(running[2] > running[0] && running[2] > running[1]);
        // Unknown (None) and settled non-success states share the neutral grey.
        let grey = menu_status_color(None);
        assert_eq!(menu_status_color(Some(PipelineStatus::Canceled)), grey);
        assert_eq!(menu_status_color(Some(PipelineStatus::Skipped)), grey);
        assert_eq!(menu_status_color(Some(PipelineStatus::Other)), grey);
        // Grey is balanced across channels, unlike the dominant-channel status colors.
        assert_eq!(grey[0], grey[1]);
        assert_eq!(grey[1], grey[2]);
    }
}
