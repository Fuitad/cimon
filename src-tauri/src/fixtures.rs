//! Dev-only fixtures mode: render the REAL native app (menu-bar tray glyph, popover panel, and the
//! settings window) against fabricated CI data so README/marketing screenshots never expose real
//! repositories.
//!
//! Activated by the `CIMON_FIXTURES` environment variable, but only on a DEVELOPER build (a debug
//! build, or one built with the `dev-tokens` feature): [`active`] is hard-wired to `None` on a
//! distributed release build, so the variable is inert there and no fixture path can ever run in a
//! shipped binary. When active, `AppState::bootstrap` seeds the in-memory config and the per-project
//! / per-account status snapshots from here (and points `config_path` at a throwaway temp file so a
//! stray save can never clobber the user's real `config.json`), the live poller is NOT spawned, and
//! the tray glyph is set once to [`FixtureData::aggregate`] -- see `commands.rs` and `lib.rs`.
//!
//! The data deliberately mirrors the frontend's browser-preview fixtures in `src/api.ts` (the same
//! `acme/*` org, `octocat`, status spread, and token-health states) so the native screenshots match
//! the documented preview states.

use std::collections::HashMap;

use crate::model::{
    Account, Config, Identity, MonitoredProject, NotificationRules, PipelineStatus, ProviderKind,
    UiMode, DEFAULT_POLL_SECS,
};
use crate::poller::{ProjectKey, ProjectStatusView, TokenHealthView};
use crate::provider::DiscoveredProject;

/// Which fabricated dataset to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// A distributed release build never constructs these: `active()` is the `None` stub there and the
// constructing parser + tests are compiled out, so "never constructed" is intentional, not dead code.
#[cfg_attr(not(any(debug_assertions, feature = "dev-tokens")), allow(dead_code))]
pub enum Mode {
    /// One GitLab account, a full status spread (failed/running/success/pending/offline/checking).
    Panel,
    /// Two accounts (GitLab + GitHub) so the panel's per-account grouping is visible.
    Multi,
    /// A dead-token account plus a token-expiring-soon account, for the auth/expiry UI.
    TokenHealth,
}

/// Which surface to bring to the foreground for capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// Hide the settings window and open the popover anchored at the top-right (the hero shot).
    Panel,
    /// Show the settings window (accounts + projects); leave the popover closed.
    Settings,
}

/// The seeded state a [`Mode`] expands into. Consumed by `AppState::bootstrap` (config + snapshots)
/// and stored back onto `AppState` (discovered + aggregate) for `list_discovered_projects` and the
/// one-shot tray glyph.
pub struct FixtureData {
    pub config: Config,
    pub project_status: HashMap<ProjectKey, ProjectStatusView>,
    pub token_health: HashMap<String, TokenHealthView>,
    pub discovered: HashMap<String, Vec<DiscoveredProject>>,
    pub aggregate: Option<PipelineStatus>,
}

/// The discovered tree + one-shot aggregate, stashed on `AppState` so the commands and the tray can
/// reach them after bootstrap. `None` in normal operation.
pub struct FixtureState {
    pub discovered: HashMap<String, Vec<DiscoveredProject>>,
    pub aggregate: Option<PipelineStatus>,
}

/// The active fixtures mode, parsed from `CIMON_FIXTURES`. DEVELOPER builds only.
#[cfg(any(debug_assertions, feature = "dev-tokens"))]
pub fn active() -> Option<Mode> {
    match std::env::var("CIMON_FIXTURES")
        .ok()?
        .trim()
        .to_lowercase()
        .as_str()
    {
        "panel" => Some(Mode::Panel),
        "multi" => Some(Mode::Multi),
        "tokenhealth" | "token-health" => Some(Mode::TokenHealth),
        _ => None,
    }
}

/// Distributed release builds never honor the fixtures variable: it is inert and this is always
/// `None`, so the seeded-data path below can never run in a shipped binary.
#[cfg(not(any(debug_assertions, feature = "dev-tokens")))]
pub fn active() -> Option<Mode> {
    None
}

/// Which surface to foreground (`CIMON_FIXTURES_SURFACE`), defaulting to the popover.
pub fn surface() -> Surface {
    match std::env::var("CIMON_FIXTURES_SURFACE")
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .as_str()
    {
        "settings" => Surface::Settings,
        _ => Surface::Panel,
    }
}

/// The forced theme (`CIMON_FIXTURES_THEME`), so a shot is reproducible regardless of the OS
/// appearance. Defaults to following the OS.
fn theme() -> UiMode {
    match std::env::var("CIMON_FIXTURES_THEME")
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .as_str()
    {
        "light" => UiMode::Light,
        "dark" => UiMode::Dark,
        _ => UiMode::System,
    }
}

/// Build the seeded state for `mode`. `now` is current UNIX seconds, used to derive recent
/// `updated_at` timestamps so the panel's relative times read sensibly ("4 min ago", not "55 years").
pub fn build(mode: Mode, now: i64) -> FixtureData {
    match mode {
        Mode::Panel => build_panel(now),
        Mode::Multi => build_multi(now),
        Mode::TokenHealth => build_token_health(now),
    }
}

// --- builders -------------------------------------------------------------------------------------

fn build_panel(now: i64) -> FixtureData {
    let projects = gitlab_acme_projects();
    // A spread of states across one account. The keys index into `projects` by id.
    let monitored = monitored_from("acc-1", &projects, &[41, 42, 46, 45, 44, 43]);

    let mut status = HashMap::new();
    put(
        &mut status,
        "acc-1",
        41,
        view(Some(PipelineStatus::Failed), "main", 4, false, now),
    );
    put(
        &mut status,
        "acc-1",
        42,
        view(
            Some(PipelineStatus::Running),
            "feature/checkout-v2",
            0,
            false,
            now,
        ),
    );
    put(
        &mut status,
        "acc-1",
        46,
        view(Some(PipelineStatus::Success), "main", 12, false, now),
    );
    put(
        &mut status,
        "acc-1",
        45,
        view(Some(PipelineStatus::Pending), "release/2.1", 1, false, now),
    );
    // A last-known success that has since gone offline (stale): shown as "passed - offline".
    put(
        &mut status,
        "acc-1",
        44,
        view(Some(PipelineStatus::Success), "main", 180, true, now),
    );
    // Project 43 (mobile-client) is intentionally ABSENT -> a neutral "checking" row.

    FixtureData {
        config: config_with(vec![gitlab_account()], monitored),
        project_status: status,
        token_health: HashMap::new(),
        discovered: HashMap::from([("acc-1".to_string(), projects)]),
        aggregate: Some(PipelineStatus::Failed),
    }
}

fn build_multi(now: i64) -> FixtureData {
    let gl = gitlab_acme_projects();
    let gh = github_acme_projects();
    let mut monitored = monitored_from("acc-1", &gl, &[41, 42, 46]);
    monitored.extend(monitored_from("gh-1", &gh, &[2001, 2002]));

    let mut status = HashMap::new();
    put(
        &mut status,
        "acc-1",
        41,
        view(Some(PipelineStatus::Failed), "main", 4, false, now),
    );
    put(
        &mut status,
        "acc-1",
        42,
        view(
            Some(PipelineStatus::Running),
            "feature/checkout-v2",
            0,
            false,
            now,
        ),
    );
    put(
        &mut status,
        "acc-1",
        46,
        view(Some(PipelineStatus::Success), "main", 12, false, now),
    );
    put(
        &mut status,
        "gh-1",
        2001,
        view(Some(PipelineStatus::Success), "main", 7, false, now),
    );
    put(
        &mut status,
        "gh-1",
        2002,
        view(
            Some(PipelineStatus::Failed),
            "fix/login-redirect",
            2,
            false,
            now,
        ),
    );

    FixtureData {
        config: config_with(vec![gitlab_account(), github_account()], monitored),
        project_status: status,
        token_health: HashMap::new(),
        discovered: HashMap::from([("acc-1".to_string(), gl), ("gh-1".to_string(), gh)]),
        aggregate: Some(PipelineStatus::Failed),
    }
}

fn build_token_health(now: i64) -> FixtureData {
    // A GitLab account with a dead token, and a GitHub account whose token expires in ~2 days.
    let dead = Account {
        id: "th-dead".into(),
        label: "Prod GitLab".into(),
        provider: ProviderKind::Gitlab,
        base_url: "https://gitlab.com".into(),
        identity: ident("ci-bot", None),
    };
    let expiring = Account {
        id: "th-exp".into(),
        label: "GitHub".into(),
        provider: ProviderKind::Github,
        base_url: "https://github.com".into(),
        identity: ident("octocat", Some("The Octocat")),
    };

    let gl = gitlab_acme_projects();
    let gh = github_acme_projects();
    let mut monitored = monitored_from("th-dead", &gl, &[41, 44]);
    monitored.extend(monitored_from("th-exp", &gh, &[2001]));

    let mut status = HashMap::new();
    // Last-known statuses are retained but the dead token makes every row on `th-dead` read
    // "authentication failed" (auth_failed is per-account and takes visual precedence).
    put(
        &mut status,
        "th-dead",
        41,
        view(Some(PipelineStatus::Failed), "main", 9, true, now),
    );
    put(
        &mut status,
        "th-dead",
        44,
        view(Some(PipelineStatus::Success), "main", 200, true, now),
    );
    put(
        &mut status,
        "th-exp",
        2001,
        view(Some(PipelineStatus::Success), "main", 5, false, now),
    );

    let mut health = HashMap::new();
    health.insert(
        "th-dead".to_string(),
        TokenHealthView {
            auth_failed: true,
            expires_at: None,
        },
    );
    health.insert(
        "th-exp".to_string(),
        TokenHealthView {
            auth_failed: false,
            expires_at: Some(date_only(now + 2 * 86_400)),
        },
    );

    FixtureData {
        config: config_with(vec![dead, expiring], monitored),
        project_status: status,
        token_health: health,
        discovered: HashMap::from([("th-dead".to_string(), gl), ("th-exp".to_string(), gh)]),
        aggregate: Some(PipelineStatus::Failed),
    }
}

// --- shared data ----------------------------------------------------------------------------------

fn gitlab_account() -> Account {
    Account {
        id: "acc-1".into(),
        label: "Work GitLab".into(),
        provider: ProviderKind::Gitlab,
        base_url: "https://gitlab.com".into(),
        identity: ident("devuser", Some("Dev User")),
    }
}

fn github_account() -> Account {
    Account {
        id: "gh-1".into(),
        // Empty label so the panel falls back to the instance host ("github.com").
        label: String::new(),
        provider: ProviderKind::Github,
        base_url: "https://github.com".into(),
        identity: ident("octocat", Some("The Octocat")),
    }
}

fn gitlab_acme_projects() -> Vec<DiscoveredProject> {
    vec![
        disc(
            41,
            "web-app",
            "https://gitlab.com/acme/frontend/web-app",
            "acme/frontend",
            None,
        ),
        disc(
            45,
            "design-system",
            "https://gitlab.com/acme/frontend/design-system",
            "acme/frontend",
            None,
        ),
        disc(
            42,
            "api-gateway",
            "https://gitlab.com/acme/backend/api-gateway",
            "acme/backend",
            None,
        ),
        disc(
            46,
            "auth-service",
            "https://gitlab.com/acme/backend/auth-service",
            "acme/backend",
            None,
        ),
        disc(
            47,
            "billing",
            "https://gitlab.com/acme/backend/billing",
            "acme/backend",
            None,
        ),
        disc(
            43,
            "mobile-client",
            "https://gitlab.com/acme/mobile/mobile-client",
            "acme/mobile",
            None,
        ),
        disc(
            44,
            "terraform",
            "https://gitlab.com/acme/ops/terraform",
            "acme/ops",
            None,
        ),
        disc(
            48,
            "dotfiles",
            "https://gitlab.com/devuser/dotfiles",
            "",
            None,
        ),
    ]
}

fn github_acme_projects() -> Vec<DiscoveredProject> {
    vec![
        disc(
            2001,
            "web-app",
            "https://github.com/acme/web-app",
            "acme",
            Some("acme/web-app"),
        ),
        disc(
            2002,
            "api",
            "https://github.com/acme/api",
            "acme",
            Some("acme/api"),
        ),
        disc(
            2003,
            "dotfiles",
            "https://github.com/octocat/dotfiles",
            "octocat",
            Some("octocat/dotfiles"),
        ),
    ]
}

// --- constructors / helpers -----------------------------------------------------------------------

fn config_with(accounts: Vec<Account>, monitored: Vec<MonitoredProject>) -> Config {
    Config {
        accounts,
        monitored,
        rules: NotificationRules::default(),
        poll_interval_secs: DEFAULT_POLL_SECS,
        launch_at_login: false,
        locale: None,
        ui_mode: theme(),
        // Pretend the menu-bar notice was already shown so a fixtures run never fires it.
        menu_bar_notice_shown: true,
        dismissed_update_version: None,
    }
}

fn ident(username: &str, name: Option<&str>) -> Identity {
    Identity {
        username: username.into(),
        name: name.map(Into::into),
        email: None,
    }
}

fn disc(
    id: u64,
    name: &str,
    web_url: &str,
    group: &str,
    remote_ref: Option<&str>,
) -> DiscoveredProject {
    DiscoveredProject {
        id,
        name: name.into(),
        web_url: web_url.into(),
        group: group.into(),
        remote_ref: remote_ref.map(Into::into),
    }
}

/// Build the monitored set for `account_id` by selecting the given ids out of a discovered list,
/// carrying name/url/remote_ref through (mirrors the frontend's `toMonitored`).
fn monitored_from(
    account_id: &str,
    discovered: &[DiscoveredProject],
    ids: &[u64],
) -> Vec<MonitoredProject> {
    ids.iter()
        .filter_map(|id| discovered.iter().find(|p| p.id == *id))
        .map(|p| MonitoredProject {
            account_id: account_id.into(),
            project_id: p.id,
            name: p.name.clone(),
            web_url: p.web_url.clone(),
            remote_ref: p.remote_ref.clone(),
        })
        .collect()
}

fn put(
    map: &mut HashMap<ProjectKey, ProjectStatusView>,
    account_id: &str,
    id: u64,
    view: ProjectStatusView,
) {
    map.insert((account_id.to_string(), id), view);
}

fn view(
    status: Option<PipelineStatus>,
    branch: &str,
    mins_ago: i64,
    stale: bool,
    now: i64,
) -> ProjectStatusView {
    ProjectStatusView {
        status,
        branch: branch.into(),
        updated_at: rfc3339(now - mins_ago * 60),
        stale,
        no_pipelines: false,
    }
}

/// Civil `(year, month, day)` for an epoch-day count (Howard Hinnant's `civil_from_days`, the
/// inverse of the one in `expiry.rs`). Self-contained so fixtures need no date crate.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Format epoch seconds as an RFC3339 UTC string (`YYYY-MM-DDTHH:MM:SSZ`), which `Date.parse` in the
/// panel accepts and renders relative.
fn rfc3339(epoch: i64) -> String {
    let (y, m, d) = civil_from_days(epoch.div_euclid(86_400));
    let secs = epoch.rem_euclid(86_400);
    let (hh, mm, ss) = (secs / 3_600, (secs % 3_600) / 60, secs % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Date-only `YYYY-MM-DD` (UTC) for epoch seconds, the GitLab token-expiry shape `days_until` parses.
fn date_only(epoch: i64) -> String {
    let (y, m, d) = civil_from_days(epoch.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_roundtrips_through_expiry_parser() {
        // The epoch we format must parse back to the same instant via the production parser.
        let epoch = 1_700_000_000; // 2023-11-14T22:13:20Z
        let s = rfc3339(epoch);
        assert_eq!(s, "2023-11-14T22:13:20Z");
        assert_eq!(crate::expiry::parse_expiry(&s), Some(epoch));
    }

    #[test]
    fn date_only_is_two_days_ahead() {
        let now = 1_700_000_000;
        // 2 days later is 2023-11-16 (UTC).
        assert_eq!(date_only(now + 2 * 86_400), "2023-11-16");
    }

    #[test]
    fn panel_fixture_has_a_failing_and_a_checking_row() {
        let fx = build(Mode::Panel, 1_700_000_000);
        assert_eq!(fx.aggregate, Some(PipelineStatus::Failed));
        assert_eq!(fx.config.monitored.len(), 6);
        // mobile-client (43) is monitored but absent from the snapshot -> a "checking" row.
        assert!(fx.config.monitored.iter().any(|m| m.project_id == 43));
        assert!(!fx.project_status.contains_key(&("acc-1".to_string(), 43)));
        // A failed row is present and drives the aggregate.
        assert_eq!(
            fx.project_status
                .get(&("acc-1".to_string(), 41))
                .unwrap()
                .status,
            Some(PipelineStatus::Failed)
        );
    }

    #[test]
    fn token_health_fixture_flags_dead_and_expiring() {
        let fx = build(Mode::TokenHealth, 1_700_000_000);
        assert!(fx.token_health.get("th-dead").unwrap().auth_failed);
        assert!(!fx.token_health.get("th-exp").unwrap().auth_failed);
        assert!(fx.token_health.get("th-exp").unwrap().expires_at.is_some());
    }

    #[test]
    fn multi_fixture_spans_two_accounts() {
        let fx = build(Mode::Multi, 1_700_000_000);
        assert_eq!(fx.config.accounts.len(), 2);
        assert!(fx.discovered.contains_key("acc-1"));
        assert!(fx.discovered.contains_key("gh-1"));
    }
}
