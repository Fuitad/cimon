//! System tray / menu-bar presence: an aggregate-status icon and a minimal right-click menu.
//!
//! Left-clicking the icon opens the custom popover panel (see `panel.rs`); right-clicking shows a
//! small native fallback menu (Open Settings, Quit) so those actions stay reachable even if the
//! webview panel ever fails to load. Labels are localized via `rust-i18n`; the tray reads the
//! GLOBAL locale, so callers MUST `i18n::apply`/`set_locale` before building or rebuilding it.

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::commands::AppState;
use crate::model::PipelineStatus;

const SETTINGS_ID: &str = "cimon-settings";
const QUIT_ID: &str = "cimon-quit";
const TRAY_ID: &str = "cimon-tray";

/// Shared RGBA status palette: vibrant, high-chroma colors (OKLCH-derived) chosen so the aggregate
/// menu-bar icon stays legible on any background, including a translucent colored menu bar where
/// muted tones would fade out. The aggregate tray icon ([`status_color`]) frames them with a thin
/// dark keyline; the per-project status colors used by the popover panel live in the frontend
/// tokens (`src/tokens.css`), tuned for window-surface contrast rather than menu-bar legibility.
const COLOR_RED: [u8; 4] = [0xFA, 0x2C, 0x2E, 0xFF]; // failed   (oklch 0.635 0.237 27)
const COLOR_BLUE: [u8; 4] = [0x00, 0x95, 0xFF, 0xFF]; // running  (oklch 0.66 0.19 250)
const COLOR_AMBER: [u8; 4] = [0xFA, 0xAD, 0x00, 0xFF]; // pending  (oklch 0.80 0.175 78)
const COLOR_GREEN: [u8; 4] = [0x00, 0xCD, 0x5E, 0xFF]; // success  (oklch 0.74 0.205 150)
const COLOR_WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF]; // idle (drawn as a macOS template)

/// RGBA color for the aggregate tray icon. `None` = idle (nothing tracked); idle is white and
/// drawn as a macOS template (see `set_status`) so the menu bar keeps it visible on any
/// background. Active states are the vibrant shared palette that [`logo_icon`] frames with a thin
/// dark keyline so the glyph reads on a dark, light, or translucent colored menu bar.
pub fn status_color(status: Option<PipelineStatus>) -> [u8; 4] {
    match status {
        Some(PipelineStatus::Failed) => COLOR_RED,
        Some(PipelineStatus::Running) => COLOR_BLUE,
        Some(PipelineStatus::Pending) | Some(PipelineStatus::Manual) => COLOR_AMBER,
        Some(_) => COLOR_GREEN, // settled/success
        None => COLOR_WHITE,    // idle
    }
}

/// Which shape the aggregate icon's central orb takes for a given status, so status is never
/// carried by color alone: a colorblind viewer, or anyone reading a grayscale screenshot, can
/// still tell the four active states apart. The outer ring and the three-dot pipeline row are
/// identical across every state ([`logo_icon`]); only the orb varies.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OrbShape {
    /// Filled disc: success/settled (the original, unchanged look).
    Solid,
    /// Hollow ring, unfilled center: pending/manual ("waiting").
    Ring,
    /// Open ~270 degree arc: running ("in motion").
    Arc,
    /// Filled disc with a diagonal knockout band: failed ("stopped").
    Slash,
}

/// Orb shape for the aggregate tray icon. Idle has no status to convey and is unused in
/// practice: it never renders outlined ([`set_status`]), so the orb shape doesn't show.
pub fn orb_shape(status: Option<PipelineStatus>) -> OrbShape {
    match status {
        Some(PipelineStatus::Failed) => OrbShape::Slash,
        Some(PipelineStatus::Running) => OrbShape::Arc,
        Some(PipelineStatus::Pending) | Some(PipelineStatus::Manual) => OrbShape::Ring,
        Some(_) => OrbShape::Solid, // settled/success
        None => OrbShape::Solid,    // idle
    }
}

/// Output size of the rendered glyph (square). The menu bar scales it to ~18pt tall, so this is
/// chosen for crisp downscaling on Retina/2-3x displays.
const ICON_N: u32 = 64;

/// Thin dark keyline drawn around the colored aggregate glyph so its silhouette stays legible on
/// any menu-bar background. `OUTLINE_STROKE` is the rim width in the 256-unit design space, kept
/// deliberately thin: a thick rim reads as a heavy black border that, by also growing the glyph's
/// footprint toward full-bleed, made the icon clash with the thin monochrome template glyphs of
/// neighboring menu-bar apps. At this width the glyph keeps the intrinsic margin of its geometry
/// (the ring's outer radius is 108 of the 128 half-box), so it sits at a neighbor-matching size.
/// The color is a near-black graphite that all but disappears into a dark bar yet still separates
/// the glyph on a light or translucent colored one (where a same-hue rim could not). Idle renders
/// without it: it is a template image macOS recolors for contrast instead.
const OUTLINE_STROKE: f64 = 5.0;
const OUTLINE_COLOR: [u8; 3] = [0x12, 0x16, 0x18];

/// Anti-aliased coverage (0.0..=1.0) of a filled disc at point `p`, centered at `c` with radius
/// `r`, where `aa` is the width (in the same units as `p`) of the soft edge band.
fn disc_coverage(p: (f64, f64), c: (f64, f64), r: f64, aa: f64) -> f64 {
    let d = ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt();
    ((r - d) / aa + 0.5).clamp(0.0, 1.0)
}

/// Anti-aliased coverage (0.0..=1.0) of an annulus (ring) at point `p`, centered at `c`, spanning
/// from `inner` to `outer` radius, with `aa` the soft-edge width. Shared by the aggregate icon's
/// outer ring and the orb's ring/arc shapes so both grow identically under keyline dilation.
fn annulus_coverage(p: (f64, f64), c: (f64, f64), outer: f64, inner: f64, aa: f64) -> f64 {
    let d = ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt();
    let outer_cov = ((outer - d) / aa + 0.5).clamp(0.0, 1.0);
    let inner_cov = ((d - inner) / aa + 0.5).clamp(0.0, 1.0);
    outer_cov.min(inner_cov)
}

/// Draw the CIMon logo glyph (outer ring + central orb + a three-dot pipeline motif) filled with
/// `color`, anti-aliased on a transparent background. Geometry mirrors the app icon
/// (`icons/*.png`) in its 256-unit design space, with a touch more mass than the icon's hairlines
/// so the ring and dots survive the menu bar's ~18pt downscale. When `outlined`, the glyph is
/// framed with a thin dark keyline ([`OUTLINE_COLOR`]) so its silhouette and the vibrant fill stay
/// legible on a dark, light, or translucent colored menu bar; this is the internal contrast the
/// flat app icon gets from its dark ring around the bright orb. Active (colored) states pass
/// `outlined = true`; the idle state is white, drawn without the keyline, and flagged as a macOS
/// template by the caller so the system recolors it for the current menu bar.
///
/// `orb` selects the central orb's shape ([`OrbShape`]). The ring and pipeline dots are identical
/// across every status; the orb is the one element with enough radius to also carry a
/// colorblind-safe shape cue alongside the fill color.
fn logo_icon(color: [u8; 4], outlined: bool, orb: OrbShape) -> tauri::image::Image<'static> {
    const N: u32 = ICON_N;
    // One output pixel expressed in the 256-unit design space; the anti-alias band width.
    let aa = 256.0 / N as f64;

    // Geometry in the 256-unit design space.
    let center = (128.0, 128.0);
    let (ring_outer, ring_inner) = (108.0, 86.0);
    let orb_c = (128.0, 116.0);
    // Orb outer footprint stays 34 for every shape, so the glyph's silhouette size doesn't change
    // when status changes; ring/arc render as a band down to ORB_BAND_INNER instead of a disc.
    const ORB_R: f64 = 34.0;
    const ORB_BAND_INNER: f64 = 22.0;
    let pipe_y = 190.0;
    let dot_xs = [78.0, 128.0, 178.0];
    let dot_r = 16.0;
    let connector_half = 5.0;

    // Coverage (0.0..=1.0) of the ring + pipeline row at point `p`, with every primitive grown by
    // `grow` design-space units. `grow = 0.0` is the fill silhouette; `grow = OUTLINE_STROKE` is
    // the dilated silhouette whose extra band becomes the dark keyline.
    let ring_and_pipeline = |p: (f64, f64), grow: f64| -> f64 {
        let ring = annulus_coverage(p, center, ring_outer + grow, ring_inner - grow, aa);
        let connector = if p.0 >= dot_xs[0] - grow && p.0 <= dot_xs[2] + grow {
            ((connector_half + grow - (p.1 - pipe_y).abs()) / aa + 0.5).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let dots = dot_xs
            .iter()
            .map(|&x| disc_coverage(p, (x, pipe_y), dot_r + grow, aa))
            .fold(0.0_f64, f64::max);
        ring.max(connector).max(dots)
    };

    // Coverage of the orb alone, shaped per `orb`. `notch` gates the failed-state knockout band:
    // it applies only to the fill layer (`grow = 0.0`), so the dilated keyline layer underneath
    // stays a solid disc and the notch reads as a dark groove rather than a gap to the background.
    let orb_coverage = |p: (f64, f64), grow: f64, notch: bool| -> f64 {
        match orb {
            OrbShape::Solid => disc_coverage(p, orb_c, ORB_R + grow, aa),
            OrbShape::Ring => annulus_coverage(p, orb_c, ORB_R + grow, ORB_BAND_INNER - grow, aa),
            OrbShape::Arc => {
                let ann = annulus_coverage(p, orb_c, ORB_R + grow, ORB_BAND_INNER - grow, aa);
                if ann <= 0.0 {
                    return 0.0;
                }
                // A 90-degree gap centered at the bottom (angle 90 in this y-down atan2), leaving
                // an open 270-degree arc that reads as motion rather than a settled ring.
                let angle = (p.1 - orb_c.1).atan2(p.0 - orb_c.0).to_degrees();
                let mut diff = (angle - 90.0).abs() % 360.0;
                if diff > 180.0 {
                    diff = 360.0 - diff;
                }
                if diff < 45.0 {
                    0.0
                } else {
                    ann
                }
            }
            OrbShape::Slash => {
                let base = disc_coverage(p, orb_c, ORB_R + grow, aa);
                if !notch || base <= 0.0 {
                    return base;
                }
                // Perpendicular distance from p to the 45-degree diagonal through the orb center;
                // inside the half-width band, knock the fill out to reveal the keyline beneath.
                const NOTCH_HALF: f64 = 6.0;
                let perp = ((p.0 - orb_c.0) - (p.1 - orb_c.1)) / std::f64::consts::SQRT_2;
                let keep = ((perp.abs() - NOTCH_HALF) / aa + 0.5).clamp(0.0, 1.0);
                base * keep
            }
        }
    };

    let coverage_at = |p: (f64, f64), grow: f64, notch: bool| -> f64 {
        ring_and_pipeline(p, grow).max(orb_coverage(p, grow, notch))
    };

    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for j in 0..N {
        for i in 0..N {
            let p = (
                (i as f64 + 0.5) * 256.0 / N as f64,
                (j as f64 + 0.5) * 256.0 / N as f64,
            );
            let fill = coverage_at(p, 0.0, true);
            let (r, g, b, a) = if outlined {
                // Composite the fill over the dark keyline over transparent (straight alpha): the
                // band the dilated silhouette adds beyond the fill is rendered in OUTLINE_COLOR.
                let outer = coverage_at(p, OUTLINE_STROKE, false);
                let out_a = fill + outer * (1.0 - fill);
                let blend = |fill_c: u8, key_c: u8| -> u8 {
                    if out_a <= 0.0 {
                        0
                    } else {
                        ((fill_c as f64 * fill + key_c as f64 * outer * (1.0 - fill)) / out_a)
                            .round()
                            .clamp(0.0, 255.0) as u8
                    }
                };
                (
                    blend(color[0], OUTLINE_COLOR[0]),
                    blend(color[1], OUTLINE_COLOR[1]),
                    blend(color[2], OUTLINE_COLOR[2]),
                    (out_a * 255.0).round() as u8,
                )
            } else {
                (color[0], color[1], color[2], (fill * 255.0).round() as u8)
            };
            rgba.push(r);
            rgba.push(g);
            rgba.push(b);
            rgba.push(a);
        }
    }
    tauri::image::Image::new_owned(rgba, N, N)
}

/// Build the tray's right-click fallback menu: just Open Settings and Quit, localized. The
/// per-project status now lives in the popover panel (left-click), so this menu carries only the
/// two actions that must stay reachable even if the webview fails to load.
fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let locale = {
        let state = app.state::<AppState>();
        let cfg = state.config.lock().unwrap();
        crate::i18n::resolve(&cfg)
    };
    let settings = MenuItemBuilder::with_id(
        SETTINGS_ID,
        &rust_i18n::t!("tray.open_settings", locale = locale),
    )
    .build(app)?;
    let quit = MenuItemBuilder::with_id(QUIT_ID, &rust_i18n::t!("tray.quit", locale = locale))
        .build(app)?;
    MenuBuilder::new(app).item(&settings).item(&quit).build()
}

/// Create the tray icon with its fallback menu and click handlers. Call once during setup.
///
/// Left-click toggles the popover panel; right-click shows the fallback menu (the menu does not
/// appear on left-click). Every tray event is forwarded to the panel so it can cache the icon
/// rect for anchoring the popover.
pub fn build_tray(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let menu = build_menu(app)?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(logo_icon(status_color(None), false, orb_shape(None)))
        // Starts idle: render the white glyph as a template so macOS keeps it visible (white on a
        // dark menu bar, dark on a light one) rather than a fixed colour that can vanish.
        .icon_as_template(true)
        .menu(&menu)
        // Left-click is reserved for the panel; the fallback menu shows on right-click only.
        .show_menu_on_left_click(false)
        .on_menu_event(|app: &AppHandle, event: tauri::menu::MenuEvent| {
            let id = event.id().as_ref();
            if id == QUIT_ID {
                app.exit(0);
            } else if id == SETTINGS_ID {
                show_settings(app);
            }
        })
        .on_tray_icon_event(|tray, event| {
            // Keep the panel's cached tray-icon rect fresh so the popover anchors correctly.
            crate::panel::on_tray_event(&event);
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::panel::toggle(tray.app_handle());
            }
        })
        .build(app)
}

/// Update the tray icon to reflect the aggregate worst status. Call from the poller.
pub fn set_status(tray: &TrayIcon, status: Option<PipelineStatus>) {
    // Active states render as vibrant, dark-keyline glyphs whose orb shape ([`orb_shape`]) plus
    // colour conveys status. Idle has no status to convey, so it renders without the keyline and
    // as a template image: macOS draws it in the menu bar's own colour (white on a dark bar, dark
    // on a light one) so it stays visible on any background.
    let _ = tray.set_icon(Some(logo_icon(
        status_color(status),
        status.is_some(),
        orb_shape(status),
    )));
    let _ = tray.set_icon_as_template(status.is_none());
}

/// Rebuild the tray's fallback menu after the locale changes (so its two items are retranslated).
pub fn refresh_menu(app: &AppHandle, tray: &TrayIcon) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    tray.set_menu(Some(menu))
}

/// Look up the live tray by id and rebuild its menu now. Called from `set_locale` so the fallback
/// menu retranslates immediately rather than on the next poll.
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
    fn logo_icon_is_tinted_glyph_with_dark_keyline() {
        // Success renders the original solid orb (unaffected by the per-status orb shape), so its
        // center stays a clean pure-fill assertion for the keyline behavior this test targets.
        let green = status_color(Some(PipelineStatus::Success));
        let img = logo_icon(green, true, orb_shape(Some(PipelineStatus::Success)));
        assert_eq!(img.width(), ICON_N);
        assert_eq!(img.height(), ICON_N);
        let rgba = img.rgba();
        let px = |x: u32, y: u32| {
            let i = ((y * ICON_N + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        };
        // Center sits inside the orb: opaque and carrying the pure status color (no keyline here).
        let c = px(ICON_N / 2, (116 * ICON_N) / 256);
        assert_eq!([c[0], c[1], c[2]], [green[0], green[1], green[2]]);
        assert!(
            c[3] > 200,
            "glyph center should be (near) opaque, got {c:?}"
        );
        // Scanning down the center column, the outermost opaque pixel is the keyline band: opaque
        // and dark. This internal contrast is what keeps the glyph legible on a colored menu bar.
        // (Scanning by coverage keeps the test valid if the glyph geometry is retuned.)
        let rim = (0..ICON_N)
            .map(|y| px(ICON_N / 2, y))
            .find(|p| p[3] > 200)
            .expect("center column should cross the glyph");
        assert!(
            rim[0] < 70 && rim[1] < 70 && rim[2] < 70,
            "outermost glyph band should be the dark keyline color, got {rim:?}"
        );
        // A corner is outside the glyph (in the design's intrinsic margin): fully transparent.
        assert_eq!(px(0, 0)[3], 0, "corner should be transparent");
    }

    #[test]
    fn idle_glyph_renders_without_a_keyline() {
        // Idle is drawn flat (no keyline) and recolored by macOS as a template image. Scanning
        // down the center column, the outermost opaque pixel must be the white fill, never a dark
        // keyline band, so macOS's template recoloring has a clean silhouette to invert.
        let img = logo_icon(status_color(None), false, orb_shape(None));
        let rgba = img.rgba();
        let px = |x: u32, y: u32| {
            let i = ((y * ICON_N + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        };
        let edge = (0..ICON_N)
            .map(|y| px(ICON_N / 2, y))
            .find(|p| p[3] > 200)
            .expect("center column should cross the idle glyph");
        assert!(
            edge[0] > 200 && edge[1] > 200 && edge[2] > 200,
            "idle glyph edge must be the white fill, not a dark keyline, got {edge:?}"
        );
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
    fn orb_shape_carries_status_independent_of_color() {
        let center_px = |img: &tauri::image::Image<'_>| -> [u8; 4] {
            let rgba = img.rgba();
            let x = ICON_N / 2;
            let y = (116 * ICON_N) / 256;
            let i = ((y * ICON_N + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        };

        // Success keeps the original solid orb: opaque and pure-color at its exact center.
        let green = status_color(Some(PipelineStatus::Success));
        let success = logo_icon(green, true, orb_shape(Some(PipelineStatus::Success)));
        let c = center_px(&success);
        assert_eq!([c[0], c[1], c[2]], [green[0], green[1], green[2]]);
        assert!(c[3] > 200, "success orb center should be opaque, got {c:?}");

        // Failed's orb is notched through its exact center: no longer the pure fill color there.
        let red = status_color(Some(PipelineStatus::Failed));
        let failed = logo_icon(red, true, orb_shape(Some(PipelineStatus::Failed)));
        let c = center_px(&failed);
        assert_ne!(
            [c[0], c[1], c[2]],
            [red[0], red[1], red[2]],
            "failed orb center should be cut by the notch, got {c:?}"
        );

        // Pending and running orbs are hollow at the exact center: transparent, not filled.
        for status in [PipelineStatus::Pending, PipelineStatus::Running] {
            let color = status_color(Some(status));
            let img = logo_icon(color, true, orb_shape(Some(status)));
            let c = center_px(&img);
            assert!(
                c[3] < 50,
                "{status:?} orb center should be hollow, got {c:?}"
            );
        }
    }
}
