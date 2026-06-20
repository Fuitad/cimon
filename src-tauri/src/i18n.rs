//! Locale resolution and application for the Rust core.
//!
//! The active locale is resolved from [`Config::locale`] -> OS locale -> English, and applied
//! process-globally via `rust_i18n::set_locale` so the background poller, the native tray, and
//! notifications all render in the chosen language without any dependency on the webview.

use crate::model::Config;

pub const DEFAULT_LOCALE: &str = "en";
pub const SUPPORTED: &[&str] = &["en", "fr"];

pub fn is_supported(code: &str) -> bool {
    SUPPORTED.contains(&code)
}

/// Resolve the active locale: explicit `Config.locale` (if a known catalog) -> OS locale (if
/// known) -> English.
pub fn resolve(cfg: &Config) -> String {
    if let Some(loc) = cfg.locale.as_deref() {
        if is_supported(loc) {
            return loc.to_string();
        }
    }
    if let Some(os) = sys_locale::get_locale() {
        // OS locale looks like "fr-CA" / "en_US"; take the language subtag.
        let short = os.split(['-', '_']).next().unwrap_or("").to_lowercase();
        if is_supported(&short) {
            return short;
        }
    }
    DEFAULT_LOCALE.to_string()
}

/// Resolve the active locale and apply it process-globally. Returns the resolved code.
///
/// MUST be called during startup BEFORE the tray is built and the poller is spawned, and again
/// after any locale change BEFORE the tray is rebuilt: tray labels read the global locale and
/// do not auto-retranslate.
pub fn apply(cfg: &Config) -> String {
    let resolved = resolve(cfg);
    rust_i18n::set_locale(&resolved);
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Config;

    fn cfg_with_locale(loc: Option<&str>) -> Config {
        Config {
            locale: loc.map(|s| s.to_string()),
            ..Config::default()
        }
    }

    #[test]
    fn translates_en_and_fr_via_per_call_override() {
        // Per-call locale override is deterministic and parallel-safe (no global state).
        assert_eq!(rust_i18n::t!("tray.quit", locale = "en"), "Quit");
        assert_eq!(rust_i18n::t!("tray.quit", locale = "fr"), "Quitter");
        assert_eq!(rust_i18n::t!("status.failed", locale = "fr"), "échoué");
    }

    #[test]
    fn unknown_locale_falls_back_to_english() {
        assert_eq!(rust_i18n::t!("tray.quit", locale = "xx"), "Quit");
    }

    #[test]
    fn interpolation_substitutes_args() {
        let msg = rust_i18n::t!("notify.pipeline_failed", locale = "en", project = "web");
        assert_eq!(msg, "web: pipeline failed");
    }

    #[test]
    fn resolve_prefers_explicit_supported_locale() {
        assert_eq!(resolve(&cfg_with_locale(Some("fr"))), "fr");
    }

    #[test]
    fn resolve_unsupported_explicit_locale_falls_through_to_known() {
        // An unsupported explicit code is ignored; resolution falls through to OS/en, both of
        // which are supported codes.
        let r = resolve(&cfg_with_locale(Some("zz")));
        assert!(
            is_supported(&r),
            "resolved {r} should be a supported locale"
        );
    }
}
