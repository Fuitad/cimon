//! Persistence of non-secret configuration as JSON in the app config directory.
//!
//! Tokens are NEVER written here, they live only in the OS keychain (see [`crate::secrets`]).
//! [`load`] validates and repairs on read so a hand-edited or corrupted file can never crash
//! or spin the poller.

use std::path::Path;

use crate::model::Config;

/// Load configuration from `path`. A missing, unreadable, or malformed file yields defaults
/// rather than an error, and any out-of-range values are repaired via [`Config::validate`].
pub fn load(path: &Path) -> Config {
    let mut cfg = match std::fs::read_to_string(path) {
        // Malformed JSON falls back to defaults rather than crashing the app.
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        // Missing or unreadable file: start from defaults.
        Err(_) => Config::default(),
    };
    cfg.validate();
    cfg
}

/// Persist configuration to `path`, creating parent directories as needed.
pub fn save(path: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(cfg).expect("Config is always serializable");
    std::fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, DEFAULT_POLL_SECS, MIN_POLL_SECS};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("cimon-cfg-{tag}-{}", std::process::id()))
    }

    #[test]
    fn load_missing_returns_default() {
        let cfg = load(Path::new(
            "/nonexistent/cimon-test-does-not-exist/config.json",
        ));
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("config.json");
        let cfg = Config {
            poll_interval_secs: 120,
            launch_at_login: true,
            ..Config::default()
        };
        save(&path, &cfg).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded, cfg);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_repairs_zero_interval_to_default() {
        let dir = temp_dir("zero");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"poll_interval_secs": 0}"#).unwrap();
        assert_eq!(load(&path).poll_interval_secs, DEFAULT_POLL_SECS);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_repairs_below_min_interval() {
        let dir = temp_dir("min");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"poll_interval_secs": 3}"#).unwrap();
        assert_eq!(load(&path).poll_interval_secs, MIN_POLL_SECS);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_malformed_returns_default_without_panicking() {
        let dir = temp_dir("malformed");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, "this is not { valid json").unwrap();
        assert_eq!(load(&path), Config::default());
        std::fs::remove_dir_all(&dir).ok();
    }
}
