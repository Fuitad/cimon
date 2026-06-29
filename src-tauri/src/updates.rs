use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::commands::{CommandError, CommandErrorKind};

pub const UPDATE_STATE_EVENT: &str = "update-state-updated";
const LATEST_JSON_ENDPOINT: &str =
    "https://github.com/Fuitad/cimon/releases/latest/download/latest.json";
const RELEASE_PAGE_URL: &str = "https://github.com/Fuitad/cimon/releases/latest";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    Idle,
    Checking,
    Available,
    UpToDate,
    Error,
    Installing,
    Installed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AvailableUpdate {
    pub version: String,
    pub body: Option<String>,
    pub date: Option<String>,
    pub release_url: String,
    pub self_updatable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateState {
    pub status: UpdateStatus,
    pub available: Option<AvailableUpdate>,
    pub last_checked_at: Option<String>,
    pub error: Option<String>,
    pub progress: Option<UpdateProgress>,
    pub dismissed_version: Option<String>,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self {
            status: UpdateStatus::Idle,
            available: None,
            last_checked_at: None,
            error: None,
            progress: None,
            dismissed_version: None,
        }
    }
}

#[derive(Clone)]
pub struct UpdateManager {
    state: Arc<Mutex<UpdateState>>,
    http: reqwest::Client,
    endpoint: String,
    /// The version a native "update available" notification was last fired for, so launch and 24h
    /// checks notify at most once per version per session instead of on every check. Runtime-only.
    last_notified: Arc<Mutex<Option<String>>>,
}

impl UpdateManager {
    pub fn new() -> Self {
        let endpoint = std::env::var("CIMON_UPDATER_ENDPOINT")
            .unwrap_or_else(|_| LATEST_JSON_ENDPOINT.to_string());
        Self {
            state: Arc::new(Mutex::new(UpdateState::default())),
            http: crate::provider::build_http_client(),
            endpoint,
            last_notified: Arc::new(Mutex::new(None)),
        }
    }

    pub fn state(&self) -> UpdateState {
        self.state.lock().unwrap().clone()
    }

    /// Seed the persisted dismissed version on startup so a dismissal made in a previous run keeps
    /// the banner hidden and suppresses notifications for that version.
    pub fn seed_dismissed_version(&self, version: Option<String>) {
        self.state.lock().unwrap().dismissed_version = version;
    }

    pub async fn check(&self, app: &tauri::AppHandle, manual: bool) -> UpdateState {
        // Never check while an install is running: a concurrent check would overwrite the
        // Installing status and zero the shared download progress.
        if self.state.lock().unwrap().status == UpdateStatus::Installing {
            return self.state();
        }
        if manual {
            self.set_status(UpdateStatus::Checking, None, None, None, app);
        }
        let result = if is_self_updatable() {
            check_self_update(app).await
        } else {
            // Compare against the Tauri app version (from tauri.conf.json), the same source the
            // self-update path uses, so Linux and macOS/Windows never disagree on "current".
            let current = app.package_info().version.to_string();
            check_manifest_update(&self.http, &self.endpoint, &current).await
        };

        match result {
            Ok(available) => self.apply_check_result(available, app),
            Err(error) if manual => self.apply_check_error(error.message, app),
            Err(_) => self.record_silent_check(),
        }
    }

    pub async fn install(&self, app: &tauri::AppHandle) -> Result<UpdateState, CommandError> {
        if !is_self_updatable() {
            return Err(CommandError::new(
                CommandErrorKind::InvalidInput,
                "this platform opens the release page instead of installing in app",
            ));
        }

        // Both the panel banner and the Settings row expose an install action, so a fast
        // double-click across the two windows could otherwise launch overlapping self-update
        // installers. Claim an exclusive install slot atomically; if one is already running,
        // return the in-flight state instead of starting a second download.
        let previous = match claim_install_slot(&self.state) {
            Some(previous) => previous,
            None => return Ok(self.state()),
        };
        let _ = app.emit(UPDATE_STATE_EVENT, self.state());
        let update = match app.updater().map_err(updater_err) {
            Ok(updater) => match updater.check().await.map_err(updater_err) {
                Ok(Some(update)) => update,
                Ok(None) => {
                    let error =
                        CommandError::new(CommandErrorKind::NotFound, "no update available");
                    return Err(self.restore_install_error(&previous, error, app));
                }
                Err(error) => return Err(self.restore_install_error(&previous, error, app)),
            },
            Err(error) => return Err(self.restore_install_error(&previous, error, app)),
        };

        let state_for_progress = self.state.clone();
        let app_for_progress = app.clone();
        let mut downloaded = 0_u64;
        let mut last_emit_pct: Option<u64> = None;
        if let Err(error) = update
            .download_and_install(
                move |chunk, total| {
                    downloaded = downloaded.saturating_add(chunk as u64);
                    let snapshot = {
                        let mut state = state_for_progress.lock().unwrap();
                        state.status = UpdateStatus::Installing;
                        state.progress = Some(UpdateProgress { downloaded, total });
                        state.clone()
                    };
                    // download_and_install fires this once per downloaded chunk (often
                    // thousands of times). Emitting on every chunk floods the webview with
                    // getUpdateState IPC refreshes and panel re-measures, so only emit when the
                    // whole-percent progress advances (the total is known for real downloads).
                    let pct = progress_percent(downloaded, total);
                    if pct != last_emit_pct {
                        last_emit_pct = pct;
                        let _ = app_for_progress.emit(UPDATE_STATE_EVENT, snapshot);
                    }
                },
                || {},
            )
            .await
            .map_err(updater_err)
        {
            return Err(self.restore_install_error(&previous, error, app));
        }

        // On macOS, download_and_install returns after swapping the app bundle, so this is the
        // completion and relaunch path. On Windows, tauri-plugin-updater runs the NSIS installer
        // and calls std::process::exit(0) inside download_and_install above, so control never
        // reaches here and the installer relaunches the app; the Installed state is therefore the
        // macOS-visible completion signal.
        let snapshot = self.set_status(UpdateStatus::Installed, None, None, None, app);
        if std::env::var_os("CIMON_UPDATER_SKIP_RESTART").is_none() {
            app.restart();
        }
        Ok(snapshot)
    }

    pub fn dismiss(&self, app: &tauri::AppHandle) -> UpdateState {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            state.dismissed_version = state.available.as_ref().map(|u| u.version.clone());
            state.clone()
        };
        let _ = app.emit(UPDATE_STATE_EVENT, snapshot.clone());
        snapshot
    }

    pub fn release_url(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .available
            .as_ref()
            .map(|u| u.release_url.clone())
            .unwrap_or_else(|| RELEASE_PAGE_URL.to_string())
    }

    fn apply_check_result(
        &self,
        available: Option<AvailableUpdate>,
        app: &tauri::AppHandle,
    ) -> UpdateState {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            state.last_checked_at = Some(now_string());
            state.error = None;
            state.progress = None;
            match available {
                Some(update) => {
                    state.status = UpdateStatus::Available;
                    state.available = Some(update);
                }
                None => {
                    state.status = UpdateStatus::UpToDate;
                    state.available = None;
                }
            }
            state.clone()
        };
        let _ = app.emit(UPDATE_STATE_EVENT, snapshot.clone());
        // Notify at most once per version per session, and never for a version the user dismissed,
        // so a standing available update does not re-alert on every launch and 24h check.
        if let Some(update) = snapshot.available.as_ref() {
            let mut last_notified = self.last_notified.lock().unwrap();
            if should_notify_version(
                &update.version,
                last_notified.as_deref(),
                snapshot.dismissed_version.as_deref(),
            ) {
                *last_notified = Some(update.version.clone());
                drop(last_notified);
                let locale = resolve_locale(app);
                crate::notify::notify_update_available(app, &update.version, &locale);
            }
        }
        snapshot
    }

    fn apply_check_error(&self, error: String, app: &tauri::AppHandle) -> UpdateState {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            state.last_checked_at = Some(now_string());
            state.status = UpdateStatus::Error;
            state.error = Some(error);
            state.progress = None;
            state.clone()
        };
        let _ = app.emit(UPDATE_STATE_EVENT, snapshot.clone());
        snapshot
    }

    /// Record that an automatic (launch/background) check ran but failed, without surfacing an
    /// error or notification. Automatic failures stay silent per the plan, but the timestamp must
    /// still advance so the state is not frozen at its pre-check default and a later successful
    /// check stays consistent.
    fn record_silent_check(&self) -> UpdateState {
        let mut state = self.state.lock().unwrap();
        state.last_checked_at = Some(now_string());
        state.clone()
    }

    fn restore_install_error(
        &self,
        previous: &UpdateState,
        error: CommandError,
        app: &tauri::AppHandle,
    ) -> CommandError {
        let snapshot = recover_install_failure_state(previous, error.message.clone());
        {
            let mut state = self.state.lock().unwrap();
            *state = snapshot.clone();
        }
        let _ = app.emit(UPDATE_STATE_EVENT, snapshot);
        error
    }

    fn set_status(
        &self,
        status: UpdateStatus,
        available: Option<AvailableUpdate>,
        error: Option<String>,
        downloaded: Option<u64>,
        app: &tauri::AppHandle,
    ) -> UpdateState {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            state.status = status;
            if let Some(available) = available {
                state.available = Some(available);
            }
            state.error = error;
            state.progress = downloaded.map(|downloaded| UpdateProgress {
                downloaded,
                total: None,
            });
            state.clone()
        };
        let _ = app.emit(UPDATE_STATE_EVENT, snapshot.clone());
        snapshot
    }
}

fn recover_install_failure_state(previous: &UpdateState, error: String) -> UpdateState {
    let mut snapshot = previous.clone();
    snapshot.status = UpdateStatus::Error;
    snapshot.error = Some(error);
    snapshot.progress = None;
    snapshot
}

/// Atomically claim the single install slot. Returns `Some(previous_state)` and marks the state
/// `Installing` when no install is in progress, or `None` when one already is, so callers can
/// refuse to start a second concurrent self-update.
fn claim_install_slot(state: &Mutex<UpdateState>) -> Option<UpdateState> {
    let mut guard = state.lock().unwrap();
    if guard.status == UpdateStatus::Installing {
        return None;
    }
    let previous = guard.clone();
    guard.status = UpdateStatus::Installing;
    guard.error = None;
    guard.progress = Some(UpdateProgress {
        downloaded: 0,
        total: None,
    });
    Some(previous)
}

/// Whole-percent download progress, or `None` when the total size is unknown. Used to throttle
/// the install progress callback so it emits at most ~100 state events instead of one per chunk.
fn progress_percent(downloaded: u64, total: Option<u64>) -> Option<u64> {
    match total {
        Some(total) if total > 0 => Some(downloaded.saturating_mul(100) / total),
        _ => None,
    }
}

/// Whether a native "update available" notification should fire for `version`: only when it was
/// not already notified this session and the user has not dismissed that exact version.
fn should_notify_version(
    version: &str,
    last_notified: Option<&str>,
    dismissed: Option<&str>,
) -> bool {
    last_notified != Some(version) && dismissed != Some(version)
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default, alias = "body")]
    notes: Option<String>,
    #[serde(default)]
    pub_date: Option<String>,
}

fn release_from_manifest(
    manifest: &str,
    current_version: &str,
) -> Result<Option<AvailableUpdate>, CommandError> {
    let manifest: Manifest = serde_json::from_str(manifest)
        .map_err(|e| CommandError::new(CommandErrorKind::InvalidInput, e.to_string()))?;
    if !is_newer_version(&manifest.version, current_version)? {
        return Ok(None);
    }
    Ok(Some(AvailableUpdate {
        version: manifest.version,
        body: manifest.notes,
        date: manifest.pub_date,
        release_url: RELEASE_PAGE_URL.to_string(),
        self_updatable: false,
    }))
}

fn is_newer_version(candidate: &str, current: &str) -> Result<bool, CommandError> {
    let candidate = semver::Version::parse(candidate.trim_start_matches('v'))
        .map_err(|e| CommandError::new(CommandErrorKind::InvalidInput, e.to_string()))?;
    let current = semver::Version::parse(current.trim_start_matches('v'))
        .map_err(|e| CommandError::new(CommandErrorKind::InvalidInput, e.to_string()))?;
    Ok(candidate > current)
}

async fn check_manifest_update(
    http: &reqwest::Client,
    endpoint: &str,
    current_version: &str,
) -> Result<Option<AvailableUpdate>, CommandError> {
    let response = http
        .get(endpoint)
        .send()
        .await
        .map_err(|e| CommandError::new(CommandErrorKind::Network, e.to_string()))?;
    if !response.status().is_success() {
        return Err(CommandError::new(
            CommandErrorKind::Http,
            format!("update manifest returned HTTP {}", response.status()),
        ));
    }
    let manifest = response
        .text()
        .await
        .map_err(|e| CommandError::new(CommandErrorKind::Network, e.to_string()))?;
    release_from_manifest(&manifest, current_version)
}

async fn check_self_update(
    app: &tauri::AppHandle,
) -> Result<Option<AvailableUpdate>, CommandError> {
    let Some(update) = app
        .updater()
        .map_err(updater_err)?
        .check()
        .await
        .map_err(updater_err)?
    else {
        return Ok(None);
    };
    let release_url = update
        .raw_json
        .get("release_url")
        .and_then(|v| v.as_str())
        .unwrap_or(RELEASE_PAGE_URL)
        .to_string();
    Ok(Some(AvailableUpdate {
        version: update.version,
        body: update.body,
        date: update.date.map(|d| d.to_string()),
        release_url,
        self_updatable: true,
    }))
}

fn is_self_updatable() -> bool {
    if std::env::var_os("CIMON_UPDATER_FORCE_NON_SELF_UPDATABLE").is_some() {
        return false;
    }
    cfg!(any(target_os = "macos", target_os = "windows"))
}

fn updater_err(e: impl std::fmt::Display) -> CommandError {
    CommandError::new(CommandErrorKind::Network, e.to_string())
}

fn now_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn resolve_locale(app: &tauri::AppHandle) -> String {
    app.try_state::<crate::commands::AppState>()
        .map(|state| {
            let cfg = state.config.lock().unwrap();
            crate::i18n::resolve(&cfg)
        })
        .unwrap_or_else(|| "en".to_string())
}

pub fn spawn_update_checks(app: tauri::AppHandle) {
    let app_for_first = app.clone();
    tauri::async_runtime::spawn(async move {
        let manager = app_for_first
            .state::<crate::commands::AppState>()
            .updates
            .clone();
        let _ = manager.check(&app_for_first, false).await;

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 60 * 60));
        // If a tick is missed (a slow check, or the process was suspended past the interval), run
        // the next check once on the following period instead of replaying a burst of missed ticks.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            let _ = manager.check(&app_for_first, false).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_from_manifest_detects_linux_update_without_platform_key() {
        let manifest = serde_json::json!({
            "version": "0.1.4",
            "notes": "Bug fixes",
            "pub_date": "2026-06-29T12:00:00Z",
            "platforms": {
                "darwin-aarch64": {
                    "signature": "sig",
                    "url": "https://example.com/cimon.app.tar.gz"
                }
            }
        })
        .to_string();

        let update = release_from_manifest(&manifest, "0.1.3")
            .unwrap()
            .expect("newer top-level version should be enough on Linux");

        assert_eq!(update.version, "0.1.4");
        assert_eq!(update.body.as_deref(), Some("Bug fixes"));
        assert!(!update.self_updatable);
        assert_eq!(update.release_url, RELEASE_PAGE_URL);
    }

    #[test]
    fn release_from_manifest_ignores_current_or_older_versions() {
        let current = r#"{"version":"0.1.3"}"#;
        let older = r#"{"version":"0.1.2"}"#;

        assert_eq!(release_from_manifest(current, "0.1.3").unwrap(), None);
        assert_eq!(release_from_manifest(older, "0.1.3").unwrap(), None);
    }

    #[test]
    fn manual_error_preserves_previous_available_metadata() {
        let manager = UpdateManager::new();
        let existing = AvailableUpdate {
            version: "0.1.4".into(),
            body: Some("Fixes".into()),
            date: None,
            release_url: RELEASE_PAGE_URL.into(),
            self_updatable: false,
        };
        {
            let mut state = manager.state.lock().unwrap();
            state.status = UpdateStatus::Available;
            state.available = Some(existing.clone());
        }

        let snapshot = {
            let mut state = manager.state.lock().unwrap();
            state.last_checked_at = Some("1".into());
            state.status = UpdateStatus::Error;
            state.error = Some("offline".into());
            state.clone()
        };

        assert_eq!(snapshot.available, Some(existing));
        assert_eq!(snapshot.status, UpdateStatus::Error);
        assert_eq!(snapshot.error.as_deref(), Some("offline"));
    }

    #[test]
    fn install_failure_recovery_keeps_available_update_and_clears_progress() {
        let existing = AvailableUpdate {
            version: "0.1.4".into(),
            body: Some("Fixes".into()),
            date: None,
            release_url: RELEASE_PAGE_URL.into(),
            self_updatable: true,
        };
        let previous = UpdateState {
            status: UpdateStatus::Available,
            available: Some(existing.clone()),
            last_checked_at: Some("123".into()),
            error: None,
            progress: Some(UpdateProgress {
                downloaded: 12,
                total: Some(100),
            }),
            dismissed_version: None,
        };

        let snapshot = recover_install_failure_state(&previous, "download failed".into());

        assert_eq!(snapshot.status, UpdateStatus::Error);
        assert_eq!(snapshot.available, Some(existing));
        assert_eq!(snapshot.progress, None);
        assert_eq!(snapshot.error.as_deref(), Some("download failed"));
    }

    #[test]
    fn claim_install_slot_blocks_a_second_concurrent_install() {
        let available = AvailableUpdate {
            version: "0.1.4".into(),
            body: None,
            date: None,
            release_url: RELEASE_PAGE_URL.into(),
            self_updatable: true,
        };
        let state = Mutex::new(UpdateState {
            status: UpdateStatus::Available,
            available: Some(available),
            last_checked_at: Some("1".into()),
            error: Some("stale".into()),
            progress: None,
            dismissed_version: None,
        });

        let first = claim_install_slot(&state).expect("first install claims the slot");
        assert_eq!(first.status, UpdateStatus::Available);
        {
            let guard = state.lock().unwrap();
            assert_eq!(guard.status, UpdateStatus::Installing);
            assert_eq!(guard.error, None);
            assert_eq!(
                guard.progress,
                Some(UpdateProgress {
                    downloaded: 0,
                    total: None,
                })
            );
        }

        assert!(
            claim_install_slot(&state).is_none(),
            "a second install must be refused while one is in progress"
        );
    }

    #[test]
    fn progress_percent_advances_only_on_whole_percent_changes() {
        assert_eq!(progress_percent(0, Some(200)), Some(0));
        assert_eq!(progress_percent(1, Some(200)), Some(0));
        assert_eq!(progress_percent(2, Some(200)), Some(1));
        assert_eq!(progress_percent(200, Some(200)), Some(100));
        assert_eq!(progress_percent(50, None), None);
        assert_eq!(progress_percent(50, Some(0)), None);
    }

    #[test]
    fn should_notify_version_dedups_session_and_dismissed_versions() {
        // Fresh version, nothing notified or dismissed yet.
        assert!(should_notify_version("0.1.4", None, None));
        // Already notified this session.
        assert!(!should_notify_version("0.1.4", Some("0.1.4"), None));
        // Dismissed by the user.
        assert!(!should_notify_version("0.1.4", None, Some("0.1.4")));
        // A newer version supersedes both a prior notification and an older dismissal.
        assert!(should_notify_version("0.1.5", Some("0.1.4"), Some("0.1.4")));
    }
}
