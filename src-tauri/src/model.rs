//! Normalized domain model shared across providers, polling, notifications and the UI.
//!
//! Provider-specific wire formats (e.g. GitLab JSON) are mapped into these types by the
//! provider implementations; nothing downstream of a `Provider` should depend on a
//! provider's raw response shape.

use serde::{Deserialize, Serialize};

/// Bounds for the poll interval (seconds). A value outside this range is a config error
/// and is clamped by [`Config::validate`].
pub const MIN_POLL_SECS: u64 = 10;
pub const MAX_POLL_SECS: u64 = 3600;
pub const DEFAULT_POLL_SECS: u64 = 30;

/// Normalized pipeline status. Provider-specific status strings map onto this set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStatus {
    Running,
    Success,
    Failed,
    Canceled,
    Skipped,
    Pending,
    Manual,
    Other,
}

impl PipelineStatus {
    /// Map a GitLab pipeline/job status string onto the normalized status.
    ///
    /// GitLab statuses: created, waiting_for_resource, preparing, pending, running,
    /// success, failed, canceled, skipped, manual, scheduled.
    pub fn from_gitlab(s: &str) -> Self {
        match s {
            "running" => PipelineStatus::Running,
            "success" => PipelineStatus::Success,
            "failed" => PipelineStatus::Failed,
            // GitLab spells it "canceled"; accept the double-l form defensively.
            "canceled" | "cancelled" => PipelineStatus::Canceled,
            "skipped" => PipelineStatus::Skipped,
            "created" | "waiting_for_resource" | "preparing" | "pending" | "scheduled" => {
                PipelineStatus::Pending
            }
            "manual" => PipelineStatus::Manual,
            _ => PipelineStatus::Other,
        }
    }

    /// Map a GitHub Actions workflow run/job onto the normalized status.
    ///
    /// GitHub carries an in-flight `status` (`queued`, `in_progress`, `completed`, plus the
    /// newer `requested`/`waiting`/`pending`) and, once `completed`, a separate `conclusion`
    /// (`success`, `failure`, `cancelled`, `skipped`, `timed_out`, `action_required`,
    /// `neutral`, `stale`, `startup_failure`). When a conclusion is present it is authoritative;
    /// otherwise the run is still in flight and the status drives the result.
    pub fn from_github(status: &str, conclusion: Option<&str>) -> Self {
        match conclusion {
            Some(c) => match c {
                "success" => PipelineStatus::Success,
                "failure" | "timed_out" | "startup_failure" => PipelineStatus::Failed,
                "cancelled" => PipelineStatus::Canceled,
                "skipped" => PipelineStatus::Skipped,
                "action_required" => PipelineStatus::Manual,
                _ => PipelineStatus::Other,
            },
            None => match status {
                "in_progress" => PipelineStatus::Running,
                "queued" | "requested" | "waiting" | "pending" => PipelineStatus::Pending,
                _ => PipelineStatus::Other,
            },
        }
    }

    /// Ranking for the aggregate tray icon: a higher value wins. Failed outranks Running,
    /// which outranks pending-like states, which outrank settled/neutral states.
    pub fn severity(&self) -> u8 {
        match self {
            PipelineStatus::Failed => 3,
            PipelineStatus::Running => 2,
            PipelineStatus::Pending | PipelineStatus::Manual => 1,
            PipelineStatus::Success
            | PipelineStatus::Canceled
            | PipelineStatus::Skipped
            | PipelineStatus::Other => 0,
        }
    }

    /// Whether a run is still in flight: queued or running, i.e. it has no terminal conclusion yet.
    /// Used to keep a multi-run commit "in progress" until every run settles, so a sibling that has
    /// already failed never pre-empts a run that is still going (see `poller::aggregate_current`).
    pub fn is_in_flight(&self) -> bool {
        matches!(self, PipelineStatus::Running | PipelineStatus::Pending)
    }

    /// rust-i18n catalog key for the user-facing status word (NOT the serde wire string).
    pub fn i18n_key(&self) -> &'static str {
        match self {
            PipelineStatus::Running => "status.running",
            PipelineStatus::Success => "status.success",
            PipelineStatus::Failed => "status.failed",
            PipelineStatus::Canceled => "status.canceled",
            PipelineStatus::Skipped => "status.skipped",
            PipelineStatus::Pending => "status.pending",
            PipelineStatus::Manual => "status.manual",
            PipelineStatus::Other => "status.other",
        }
    }
}

/// Reuse the pipeline status set for jobs; GitLab uses the same vocabulary for both.
pub type JobStatus = PipelineStatus;

/// A normalized CI pipeline (a GitLab pipeline, later a GitHub workflow run).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: u64,
    pub project_id: u64,
    pub status: PipelineStatus,
    /// The git ref (branch/tag). `ref` is a Rust keyword, hence the trailing underscore.
    #[serde(rename = "ref")]
    pub ref_: String,
    pub sha: String,
    pub web_url: String,
    pub updated_at: String,
    /// Whether multiple pipelines sharing this run's `sha` are the SAME trigger fanning out (e.g. one
    /// GitHub push firing several workflow files simultaneously) and should be aggregated together,
    /// worst status across the group wins (see `poller::aggregate_current`). GitLab pipelines are each
    /// an independently triggered, self-contained result: a schedule- or api-triggered pipeline runs
    /// against the branch's CURRENT head sha, so it can share a sha with an unrelated pipeline hours or
    /// days earlier simply because no new commit landed in between. Grouping those would let a stale
    /// scheduled failure redden a later, unrelated pass, so GitLab sets this `false`.
    pub commit_fanout: bool,
}

/// A normalized job within a pipeline (a GitLab job, later a GitHub workflow job).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub status: JobStatus,
    pub stage: String,
    /// The job's own page (GitLab job `web_url`, GitHub Actions job `html_url`). A clicked
    /// job-level notification opens this; pipeline-level notifications open the pipeline's URL.
    pub web_url: String,
}

/// Which CI provider an account talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Gitlab,
    Github,
}

/// The authenticated identity resolved when a token is validated. Never contains the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub username: String,
    pub name: Option<String>,
    pub email: Option<String>,
}

/// A configured account. Contains NO token: the token lives only in the OS keychain,
/// keyed by `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub label: String,
    pub provider: ProviderKind,
    pub base_url: String,
    pub identity: Identity,
}

/// A project the user has chosen to monitor. Account-scoped, because a GitLab project id
/// is unique only within its instance/account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitoredProject {
    pub account_id: String,
    pub project_id: u64,
    pub name: String,
    pub web_url: String,
    /// Provider-specific project ADDRESS used when `project_id` is not the API address.
    /// `Some("owner/repo")` for GitHub; `None` for GitLab (which addresses by `project_id`).
    /// Not a git ref (see `Pipeline::ref_`). No `serde(default)`: pre-beta, so a config written
    /// before this field fails to load and resets to defaults rather than being migrated.
    pub remote_ref: Option<String>,
}

/// Global notification preferences (v1: one set applies to all monitored projects).
///
/// Pipeline and job events are configured independently: the `on_*` booleans gate pipeline
/// transitions and the `job_on_*` booleans gate individual-job transitions. A transition notifies
/// when its matching toggle is on (see `notify::should_notify`). `#[serde(default)]` lets configs
/// written before this change still load: the older `pipeline_level` / `job_level` / `detail_level`
/// fields are unknown now and ignored, and the missing `job_on_*` bools fall back to off below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationRules {
    pub on_start: bool,
    pub on_success: bool,
    pub on_fail: bool,
    /// Notify when an individual job starts.
    pub job_on_start: bool,
    /// Notify when an individual job succeeds.
    pub job_on_success: bool,
    /// Notify when an individual job fails.
    pub job_on_fail: bool,
}

impl Default for NotificationRules {
    fn default() -> Self {
        // Quiet-ish default: notify on pipeline completion (success/fail), not on every start;
        // job events are all opt-in to avoid flooding the user with per-job noise.
        NotificationRules {
            on_start: false,
            on_success: true,
            on_fail: true,
            job_on_start: false,
            job_on_success: false,
            job_on_fail: false,
        }
    }
}

impl NotificationRules {
    /// Whether any job event is enabled. Gates whether the poller fetches per-job status at all:
    /// with no job event on, there is no reason to pay for the extra per-pipeline jobs request.
    pub fn any_job_enabled(&self) -> bool {
        self.job_on_start || self.job_on_success || self.job_on_fail
    }
}

/// User-selected color theme for the app windows. `System` follows the OS appearance
/// (`prefers-color-scheme`); `Light`/`Dark` force the palette regardless of the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UiMode {
    #[default]
    System,
    Light,
    Dark,
}

/// Persisted, non-secret configuration. Tokens are NEVER stored here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub accounts: Vec<Account>,
    pub monitored: Vec<MonitoredProject>,
    pub rules: NotificationRules,
    pub poll_interval_secs: u64,
    pub launch_at_login: bool,
    /// Active UI/notification locale. `None` means "follow the OS, else English".
    /// Single source of truth shared by the Rust core and the frontend.
    pub locale: Option<String>,
    /// Color theme for the app windows. Defaults to following the OS.
    pub ui_mode: UiMode,
    /// Whether the one-time "CIMon is running in your menu bar" notice has been shown. Set the
    /// first time the app starts hidden (accounts configured) so the notice never nags again.
    pub menu_bar_notice_shown: bool,
    /// The update version the user dismissed, persisted so dismissing an update banner survives a
    /// restart. `None` means nothing is dismissed. Internal-only, not exposed to the frontend.
    pub dismissed_update_version: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            accounts: Vec::new(),
            monitored: Vec::new(),
            rules: NotificationRules::default(),
            poll_interval_secs: DEFAULT_POLL_SECS,
            launch_at_login: false,
            locale: None,
            ui_mode: UiMode::System,
            menu_bar_notice_shown: false,
            dismissed_update_version: None,
        }
    }
}

impl Config {
    /// Clamp/repair any out-of-range values so a hand-edited or corrupted config can never
    /// crash or spin the poller. Used by both load and the command setters.
    pub fn validate(&mut self) {
        self.poll_interval_secs = clamp_interval(self.poll_interval_secs);
    }
}

/// Clamp a poll interval into the supported range. A `0` (or anything below the floor)
/// becomes the default rather than the floor, since `0` almost always means "unset".
pub fn clamp_interval(secs: u64) -> u64 {
    if secs == 0 {
        DEFAULT_POLL_SECS
    } else {
        secs.clamp(MIN_POLL_SECS, MAX_POLL_SECS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_github_maps_status_and_conclusion() {
        // Completed runs map by conclusion.
        assert_eq!(
            PipelineStatus::from_github("completed", Some("success")),
            PipelineStatus::Success
        );
        assert_eq!(
            PipelineStatus::from_github("completed", Some("failure")),
            PipelineStatus::Failed
        );
        for failed in ["failure", "timed_out", "startup_failure"] {
            assert_eq!(
                PipelineStatus::from_github("completed", Some(failed)),
                PipelineStatus::Failed,
                "{failed} should map to Failed"
            );
        }
        assert_eq!(
            PipelineStatus::from_github("completed", Some("cancelled")),
            PipelineStatus::Canceled
        );
        assert_eq!(
            PipelineStatus::from_github("completed", Some("skipped")),
            PipelineStatus::Skipped
        );
        assert_eq!(
            PipelineStatus::from_github("completed", Some("action_required")),
            PipelineStatus::Manual
        );
        for other in ["neutral", "stale", "something_new"] {
            assert_eq!(
                PipelineStatus::from_github("completed", Some(other)),
                PipelineStatus::Other,
                "{other} conclusion should map to Other"
            );
        }
        // In-flight runs (no conclusion yet) map by status.
        assert_eq!(
            PipelineStatus::from_github("in_progress", None),
            PipelineStatus::Running
        );
        for pending in ["queued", "requested", "waiting", "pending"] {
            assert_eq!(
                PipelineStatus::from_github(pending, None),
                PipelineStatus::Pending,
                "{pending} should map to Pending"
            );
        }
        assert_eq!(
            PipelineStatus::from_github("something_new", None),
            PipelineStatus::Other
        );
    }

    #[test]
    fn provider_kind_github_serializes_to_github() {
        assert_eq!(
            serde_json::to_string(&ProviderKind::Github).unwrap(),
            "\"github\""
        );
        assert_eq!(
            serde_json::from_str::<ProviderKind>("\"github\"").unwrap(),
            ProviderKind::Github
        );
    }

    #[test]
    fn from_gitlab_maps_all_documented_statuses() {
        assert_eq!(
            PipelineStatus::from_gitlab("running"),
            PipelineStatus::Running
        );
        assert_eq!(
            PipelineStatus::from_gitlab("success"),
            PipelineStatus::Success
        );
        assert_eq!(
            PipelineStatus::from_gitlab("failed"),
            PipelineStatus::Failed
        );
        assert_eq!(
            PipelineStatus::from_gitlab("canceled"),
            PipelineStatus::Canceled
        );
        assert_eq!(
            PipelineStatus::from_gitlab("skipped"),
            PipelineStatus::Skipped
        );
        assert_eq!(
            PipelineStatus::from_gitlab("manual"),
            PipelineStatus::Manual
        );
        for pending in [
            "created",
            "waiting_for_resource",
            "preparing",
            "pending",
            "scheduled",
        ] {
            assert_eq!(
                PipelineStatus::from_gitlab(pending),
                PipelineStatus::Pending,
                "{pending} should map to Pending"
            );
        }
    }

    #[test]
    fn from_gitlab_unknown_maps_to_other() {
        assert_eq!(
            PipelineStatus::from_gitlab("something_new"),
            PipelineStatus::Other
        );
        assert_eq!(PipelineStatus::from_gitlab(""), PipelineStatus::Other);
    }

    #[test]
    fn severity_orders_failed_above_running_above_rest() {
        assert!(PipelineStatus::Failed.severity() > PipelineStatus::Running.severity());
        assert!(PipelineStatus::Running.severity() > PipelineStatus::Pending.severity());
        assert!(PipelineStatus::Pending.severity() > PipelineStatus::Success.severity());
        // Settled/neutral states all rank at the bottom.
        assert_eq!(
            PipelineStatus::Success.severity(),
            PipelineStatus::Skipped.severity()
        );
    }

    #[test]
    fn clamp_interval_repairs_out_of_range() {
        assert_eq!(clamp_interval(0), DEFAULT_POLL_SECS);
        assert_eq!(clamp_interval(5), MIN_POLL_SECS);
        assert_eq!(clamp_interval(100), 100);
        assert_eq!(clamp_interval(99_999), MAX_POLL_SECS);
    }

    #[test]
    fn config_locale_roundtrips_and_defaults_none() {
        let mut cfg = Config::default();
        assert_eq!(cfg.locale, None);
        cfg.locale = Some("fr".to_string());
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.locale.as_deref(), Some("fr"));
        // Configs written before the locale field still load (serde default).
        let old: Config = serde_json::from_str(r#"{"poll_interval_secs":30}"#).unwrap();
        assert_eq!(old.locale, None);
        // The menu-bar notice flag defaults off for a config that predates it, so an existing
        // user sees the notice once on their next hidden launch rather than never.
        assert!(!old.menu_bar_notice_shown);
    }

    #[test]
    fn config_ui_mode_roundtrips_and_defaults_system() {
        let mut cfg = Config::default();
        assert_eq!(cfg.ui_mode, UiMode::System);
        cfg.ui_mode = UiMode::Dark;
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"ui_mode\":\"dark\""));
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ui_mode, UiMode::Dark);
        // A config written before the ui_mode field still loads and defaults to following the OS.
        let old: Config = serde_json::from_str(r#"{"poll_interval_secs":30}"#).unwrap();
        assert_eq!(old.ui_mode, UiMode::System);
    }

    #[test]
    fn notification_rules_roundtrip_and_old_config_loads() {
        let rules = NotificationRules {
            on_start: true,
            on_success: false,
            on_fail: true,
            job_on_start: true,
            job_on_success: false,
            job_on_fail: true,
        };
        let json = serde_json::to_string(&rules).unwrap();
        assert_eq!(
            serde_json::from_str::<NotificationRules>(&json).unwrap(),
            rules
        );

        // A config written before this change carried the old detail-level model (`pipeline_level`
        // / `job_level`, or an even older single `detail_level` string). It must still load: the
        // now-unknown fields are ignored and the missing `job_on_*` bools fall back to off. The
        // pipeline `on_*` events are preserved as written; legacy `job_level: true` deliberately
        // does NOT carry over to the new per-event job toggles (simple-migration behavior).
        let old: NotificationRules = serde_json::from_str(
            r#"{"on_start":false,"on_success":true,"on_fail":true,"pipeline_level":false,"job_level":true,"detail_level":"both"}"#,
        )
        .unwrap();
        assert!(
            old.on_success,
            "pipeline success event preserved from old config"
        );
        assert!(old.on_fail, "pipeline fail event preserved from old config");
        assert!(
            !old.any_job_enabled(),
            "legacy job_level does not enable the new per-event job toggles"
        );
    }

    #[test]
    fn config_serializes_without_token_field() {
        // Guard for Truth 3: no token/secret field ever appears in the serialized config.
        let cfg = Config::default();
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.to_lowercase().contains("token"));
        assert!(!json.to_lowercase().contains("secret"));
        assert!(!json.to_lowercase().contains("password"));
    }
}
