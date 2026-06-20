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
}

/// A normalized job within a pipeline (a GitLab job, later a GitHub workflow job).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub status: JobStatus,
    pub stage: String,
}

/// Which CI provider an account talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Gitlab,
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
}

/// Global notification preferences (v1: one set applies to all monitored projects).
///
/// Detail level is two INDEPENDENT toggles, not a mutually-exclusive choice: the PRD specifies
/// "either/both detail levels". A transition notifies only when its event type AND its detail
/// level are both enabled. `#[serde(default)]` lets a config written before job-level (which
/// had a single `detail_level` field) still load: the unknown field is ignored and the missing
/// bools fall back to the defaults below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationRules {
    pub on_start: bool,
    pub on_success: bool,
    pub on_fail: bool,
    /// Notify on pipeline-level transitions.
    pub pipeline_level: bool,
    /// Notify on job-level transitions (individual jobs within a pipeline).
    pub job_level: bool,
}

impl Default for NotificationRules {
    fn default() -> Self {
        // Quiet-ish default: notify on completion (success/fail), not on every start, and at
        // pipeline level only (job-level is opt-in to avoid flooding the user with per-job noise).
        NotificationRules {
            on_start: false,
            on_success: true,
            on_fail: true,
            pipeline_level: true,
            job_level: false,
        }
    }
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
    /// Whether the one-time "CIMon is running in your menu bar" notice has been shown. Set the
    /// first time the app starts hidden (accounts configured) so the notice never nags again.
    pub menu_bar_notice_shown: bool,
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
            menu_bar_notice_shown: false,
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
    fn notification_rules_roundtrip_and_old_config_loads() {
        let rules = NotificationRules {
            on_start: true,
            on_success: false,
            on_fail: true,
            pipeline_level: false,
            job_level: true,
        };
        let json = serde_json::to_string(&rules).unwrap();
        assert_eq!(
            serde_json::from_str::<NotificationRules>(&json).unwrap(),
            rules
        );

        // A config written before job-level carried a single `detail_level` field and no
        // level bools. It must still load: the unknown field is ignored and the missing bools
        // fall back to the defaults (pipeline on, job off).
        let old: NotificationRules = serde_json::from_str(
            r#"{"on_start":false,"on_success":true,"on_fail":true,"detail_level":"pipeline"}"#,
        )
        .unwrap();
        assert!(
            old.pipeline_level,
            "pipeline-level defaults on for an old config"
        );
        assert!(!old.job_level, "job-level defaults off for an old config");
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
