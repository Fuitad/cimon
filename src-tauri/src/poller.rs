//! Background polling and pure transition detection.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::model::{Config, Job, Pipeline, PipelineStatus, MIN_POLL_SECS};
use crate::provider::{build_provider, Provider};
use crate::secrets::TokenStore;

/// `(account_id, project_id)` uniquely identifies a monitored project across accounts. A GitLab
/// project id is only unique within its instance/account, hence the account in the key.
pub type ProjectKey = (String, u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    Started,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub pipeline: Pipeline,
    pub kind: TransitionKind,
    /// `Some` for a job-level transition (carries the job that moved); `None` for a
    /// pipeline-level transition. Drives both the detail-toggle filter and the message.
    pub job: Option<Job>,
    /// Account that owns the project (filled by `poll_once`; empty from `detect` alone).
    pub account_id: String,
    /// Display name of the monitored project (filled by `poll_once`).
    pub project_name: String,
}

/// Map a pipeline status to the transition kind worth notifying about (if any).
fn transition_for(status: PipelineStatus) -> Option<TransitionKind> {
    match status {
        PipelineStatus::Running => Some(TransitionKind::Started),
        PipelineStatus::Success => Some(TransitionKind::Succeeded),
        PipelineStatus::Failed => Some(TransitionKind::Failed),
        _ => None,
    }
}

/// Last-seen pipeline state, used to detect transitions across polls.
#[derive(Default)]
pub struct PollState {
    /// Last-seen status per pipeline id, per project.
    seen: HashMap<ProjectKey, HashMap<u64, PipelineStatus>>,
    /// Status of the most recent pipeline per project (drives the aggregate tray icon).
    current: HashMap<ProjectKey, PipelineStatus>,
    /// Last-seen status per job id, per project. Only the currently-tracked (newest) pipeline's
    /// jobs are kept (pruned each tick), so this stays bounded to one pipeline's worth per project.
    seen_jobs: HashMap<ProjectKey, HashMap<u64, PipelineStatus>>,
}

impl PollState {
    /// Diff the latest pipelines for a project against last-seen state, returning the
    /// transitions to notify about.
    ///
    /// The FIRST observation of a project (including a project newly added to the monitored set
    /// while the poller is already running) seeds state and returns nothing, so pipelines that
    /// predate monitoring never produce notifications.
    ///
    /// `latest` is expected newest-first (the GitLab list is ordered by `updated_at` desc), so
    /// `latest[0]` drives the project's current aggregate status.
    pub fn detect(&mut self, key: &ProjectKey, latest: &[Pipeline]) -> Vec<Transition> {
        // The newest pipeline (list is sorted desc) drives this project's aggregate status;
        // an empty list means the project has no current pipeline, so drop any stale entry.
        match latest.first() {
            Some(newest) => {
                self.current.insert(key.clone(), newest.status);
            }
            None => {
                self.current.remove(key);
            }
        }

        let is_first = !self.seen.contains_key(key);
        let prev = self.seen.entry(key.clone()).or_default();

        if is_first {
            // Baseline: record everything, emit nothing (no notifications for pre-existing runs).
            for p in latest {
                prev.insert(p.id, p.status);
            }
            return Vec::new();
        }

        let mut out = Vec::new();
        for p in latest {
            let changed = match prev.get(&p.id) {
                None => true,                  // a pipeline that started after the baseline
                Some(&old) => old != p.status, // an existing pipeline whose status moved
            };
            if changed {
                if let Some(kind) = transition_for(p.status) {
                    // account_id / project_name are filled by poll_once, which knows the
                    // monitored project context; detect only sees the pipeline list.
                    out.push(Transition {
                        pipeline: p.clone(),
                        kind,
                        job: None,
                        account_id: String::new(),
                        project_name: String::new(),
                    });
                }
                prev.insert(p.id, p.status);
            }
        }
        out
    }

    /// Diff a pipeline's jobs against last-seen state, returning the job-level transitions to
    /// notify about. The FIRST time jobs are observed for a project, they are baselined (seeded,
    /// nothing emitted) so jobs already in flight never notify. The baseline gate is keyed on
    /// job state specifically (not the pipeline baseline): enabling the job-level toggle
    /// mid-session therefore baselines the in-flight jobs instead of flooding the user with a
    /// notification for every job that happened to be running at the moment it was turned on.
    ///
    /// Only the most-recently-updated pipeline's jobs are passed in (see `poll_once`), so the
    /// per-project job map is pruned to the current job ids each tick to stay bounded.
    pub fn detect_jobs(
        &mut self,
        key: &ProjectKey,
        pipeline: &Pipeline,
        jobs: &[Job],
    ) -> Vec<Transition> {
        let is_first = !self.seen_jobs.contains_key(key);
        let prev = self.seen_jobs.entry(key.clone()).or_default();
        if is_first {
            for j in jobs {
                prev.insert(j.id, j.status);
            }
            return Vec::new();
        }

        let mut out = Vec::new();
        let mut current_ids = HashSet::new();
        for j in jobs {
            current_ids.insert(j.id);
            let changed = match prev.get(&j.id) {
                None => true,                  // a job not seen since the baseline (new pipeline/job)
                Some(&old) => old != j.status, // an existing job whose status moved
            };
            if changed {
                if let Some(kind) = transition_for(j.status) {
                    out.push(Transition {
                        pipeline: pipeline.clone(),
                        kind,
                        job: Some(j.clone()),
                        account_id: String::new(),
                        project_name: String::new(),
                    });
                }
                prev.insert(j.id, j.status);
            }
        }
        // Drop ids no longer present so superseded pipelines' jobs don't accumulate.
        prev.retain(|id, _| current_ids.contains(id));
        out
    }

    /// Worst current status across all monitored projects, or `None` when nothing is tracked.
    /// Drives the tray icon (Failed outranks Running outranks settled states).
    pub fn aggregate_status(&self) -> Option<PipelineStatus> {
        self.current.values().copied().max_by_key(|s| s.severity())
    }

    /// Drop tracked state for keys no longer in the monitored set, so an un-monitored project
    /// stops driving the aggregate and its transition history is forgotten.
    pub fn retain(&mut self, valid: &HashSet<ProjectKey>) {
        self.seen.retain(|k, _| valid.contains(k));
        self.current.retain(|k, _| valid.contains(k));
        self.seen_jobs.retain(|k, _| valid.contains(k));
    }
}

/// Poll every monitored project once, returning all detected transitions. A fetch error for one
/// project is isolated: it is skipped this tick and does not affect other projects or the loop.
pub async fn poll_once(
    state: &mut PollState,
    http: &reqwest::Client,
    tokens: &dyn TokenStore,
    cfg: &Config,
) -> Vec<Transition> {
    let mut transitions = Vec::new();
    for acct in &cfg.accounts {
        // Nothing to poll for this account: skip it WITHOUT reading the keychain. Otherwise we
        // would request token access every tick for an account with no monitored projects, which
        // on macOS triggers a recurring keychain authorization prompt for no benefit.
        if !cfg.monitored.iter().any(|m| m.account_id == acct.id) {
            continue;
        }
        let token = match tokens.get(&acct.id) {
            Ok(Some(t)) => t,
            _ => continue, // no/unreadable token: skip this account this tick
        };
        let provider = build_provider(
            acct.provider,
            http.clone(),
            acct.base_url.clone(),
            token.expose().to_string(),
        );
        for mp in cfg.monitored.iter().filter(|m| m.account_id == acct.id) {
            let pipelines = match provider
                .list_pipelines(mp.project_id, mp.remote_ref.as_deref())
                .await
            {
                Ok(p) => p,
                Err(_) => continue, // error isolation: skip this project this tick
            };
            let key = (acct.id.clone(), mp.project_id);
            let mut detected = state.detect(&key, &pipelines);

            // Job-level: fetch jobs only for the newest (most-recently-updated) pipeline, so
            // cost is at most one extra request per project per tick. A job-fetch error is
            // isolated (skip job detection this tick) just like a pipeline-fetch error.
            if cfg.rules.job_level {
                if let Some(newest) = pipelines.first() {
                    if let Ok(jobs) = provider
                        .list_jobs(mp.project_id, mp.remote_ref.as_deref(), newest.id)
                        .await
                    {
                        detected.extend(state.detect_jobs(&key, newest, &jobs));
                    }
                }
            }

            for tr in &mut detected {
                tr.account_id = acct.id.clone();
                tr.project_name = mp.name.clone();
            }
            transitions.extend(detected);
        }
    }
    // Forget state for projects/accounts no longer monitored so the tray aggregate and
    // transition history don't include stale entries after a project is un-monitored.
    let valid: HashSet<ProjectKey> = cfg
        .monitored
        .iter()
        .map(|m| (m.account_id.clone(), m.project_id))
        .collect();
    state.retain(&valid);
    transitions
}

/// Run the polling loop until the task is dropped. Each tick reads the current config (so
/// changes to the monitored set / interval take effect), polls once, and forwards transitions
/// and the new aggregate status to the callbacks. Spawned during app setup (Task 11).
pub async fn run_poller<F, G>(
    http: reqwest::Client,
    tokens: Arc<dyn TokenStore>,
    config: Arc<Mutex<Config>>,
    mut on_transitions: F,
    mut on_aggregate: G,
) where
    F: FnMut(&[Transition]),
    G: FnMut(Option<PipelineStatus>),
{
    let mut state = PollState::default();
    loop {
        let cfg = config.lock().unwrap().clone();
        // Defensive floor in case an invalid value ever slips past validation.
        let interval_secs = cfg.poll_interval_secs.max(MIN_POLL_SECS);
        let transitions = poll_once(&mut state, &http, &*tokens, &cfg).await;
        if !transitions.is_empty() {
            on_transitions(&transitions);
        }
        on_aggregate(state.aggregate_status());
        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Account;
    use crate::provider::build_http_client;
    use crate::secrets::MemoryTokenStore;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn poll_once_dispatches_to_github_provider() {
        let server = MockServer::start().await;
        // GitHub Actions runs endpoint (GHE path, since server.uri() is an IP host). A GitLab
        // provider would request `/api/v4/...` instead, so reaching this path AND parsing the
        // run proves the dispatch routed to GithubProvider.
        Mock::given(method("GET"))
            .and(path("/api/v3/repos/acme/web-app/actions/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_count": 1,
                "workflow_runs": [
                    {"id": 7, "head_branch": "main", "head_sha": "a", "status": "completed",
                     "conclusion": "failure", "html_url": "http://x/7", "updated_at": "t"}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let tokens = MemoryTokenStore::new();
        tokens.store("gh", "tok").unwrap();
        let cfg = Config {
            accounts: vec![Account {
                id: "gh".into(),
                label: "l".into(),
                provider: crate::model::ProviderKind::Github,
                base_url: server.uri(),
                identity: crate::model::Identity {
                    username: "u".into(),
                    name: None,
                    email: None,
                },
            }],
            monitored: vec![crate::model::MonitoredProject {
                account_id: "gh".into(),
                project_id: 7,
                name: "web-app".into(),
                web_url: "http://x".into(),
                remote_ref: Some("acme/web-app".into()),
            }],
            ..Config::default()
        };

        let mut state = PollState::default();
        let first = poll_once(&mut state, &build_http_client(), &tokens, &cfg).await;
        assert!(first.is_empty(), "first poll baselines, emits nothing");
        // The run was fetched, parsed and status-mapped through GithubProvider.
        assert_eq!(state.aggregate_status(), Some(PipelineStatus::Failed));
    }

    fn pipeline(id: u64, status: PipelineStatus) -> Pipeline {
        Pipeline {
            id,
            project_id: 1,
            status,
            ref_: "main".into(),
            sha: "abc".into(),
            web_url: format!("http://x/{id}"),
            updated_at: "2026-06-20T00:00:00Z".into(),
        }
    }

    fn job(id: u64, status: PipelineStatus) -> Job {
        Job {
            id,
            name: format!("job{id}"),
            status,
            stage: "test".into(),
        }
    }

    fn key() -> ProjectKey {
        ("acct".into(), 1)
    }

    #[test]
    fn first_observation_is_baseline_no_transitions() {
        let mut s = PollState::default();
        let out = s.detect(&key(), &[pipeline(1, PipelineStatus::Running)]);
        assert!(out.is_empty(), "first poll must seed, not notify");
    }

    #[test]
    fn new_running_pipeline_after_baseline_emits_started() {
        let mut s = PollState::default();
        s.detect(&key(), &[pipeline(1, PipelineStatus::Success)]); // baseline
        let out = s.detect(
            &key(),
            &[
                pipeline(2, PipelineStatus::Running),
                pipeline(1, PipelineStatus::Success),
            ],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, TransitionKind::Started);
        assert_eq!(out[0].pipeline.id, 2);
    }

    #[test]
    fn status_change_to_terminal_emits_succeeded_or_failed() {
        let mut s = PollState::default();
        s.detect(&key(), &[pipeline(1, PipelineStatus::Running)]); // baseline
        let out = s.detect(&key(), &[pipeline(1, PipelineStatus::Failed)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, TransitionKind::Failed);
    }

    #[test]
    fn unchanged_status_emits_nothing() {
        let mut s = PollState::default();
        s.detect(&key(), &[pipeline(1, PipelineStatus::Running)]);
        let out = s.detect(&key(), &[pipeline(1, PipelineStatus::Running)]);
        assert!(out.is_empty());
    }

    #[test]
    fn project_added_mid_run_seeds_without_notifying() {
        // A project the poller has never seen before (e.g. just toggled on by the user) must
        // baseline on its first observation, even with pre-existing running pipelines.
        let mut s = PollState::default();
        // Some other project already tracked:
        s.detect(
            &("other".into(), 9),
            &[pipeline(50, PipelineStatus::Failed)],
        );
        // Newly-added project's first observation: pre-existing running pipeline, no notify.
        let out = s.detect(&("new".into(), 2), &[pipeline(99, PipelineStatus::Running)]);
        assert!(
            out.is_empty(),
            "newly added project must baseline, not emit Started"
        );
    }

    #[test]
    fn aggregate_status_reflects_worst_current() {
        let mut s = PollState::default();
        assert_eq!(s.aggregate_status(), None);
        s.detect(&("a".into(), 1), &[pipeline(1, PipelineStatus::Running)]);
        s.detect(&("b".into(), 2), &[pipeline(2, PipelineStatus::Failed)]);
        assert_eq!(s.aggregate_status(), Some(PipelineStatus::Failed));
    }

    #[test]
    fn empty_pipeline_list_clears_current_status() {
        let mut s = PollState::default();
        s.detect(&key(), &[pipeline(1, PipelineStatus::Failed)]);
        assert_eq!(s.aggregate_status(), Some(PipelineStatus::Failed));
        // The project's pipelines are gone: it must stop driving the aggregate.
        s.detect(&key(), &[]);
        assert_eq!(s.aggregate_status(), None);
    }

    #[test]
    fn retain_drops_unmonitored_projects() {
        let mut s = PollState::default();
        s.detect(&("a".into(), 1), &[pipeline(1, PipelineStatus::Failed)]);
        s.detect(&("a".into(), 2), &[pipeline(2, PipelineStatus::Running)]);
        assert_eq!(s.aggregate_status(), Some(PipelineStatus::Failed));
        // Project 1 is un-monitored; only (a, 2) remains valid.
        let valid: HashSet<ProjectKey> = [("a".to_string(), 2u64)].into_iter().collect();
        s.retain(&valid);
        assert_eq!(s.aggregate_status(), Some(PipelineStatus::Running));
    }

    #[test]
    fn job_first_observation_is_baseline_no_transitions() {
        let mut s = PollState::default();
        let p = pipeline(1, PipelineStatus::Running);
        let out = s.detect_jobs(&key(), &p, &[job(10, PipelineStatus::Running)]);
        assert!(
            out.is_empty(),
            "first job observation must baseline, not notify"
        );
    }

    #[test]
    fn job_started_then_terminal_emits_after_baseline() {
        let mut s = PollState::default();
        let p = pipeline(1, PipelineStatus::Running);
        s.detect_jobs(&key(), &p, &[]); // first job observation baselines (none yet)

        let started = s.detect_jobs(&key(), &p, &[job(10, PipelineStatus::Running)]);
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].kind, TransitionKind::Started);
        assert_eq!(
            started[0].job.as_ref().unwrap().id,
            10,
            "transition carries the job"
        );

        let failed = s.detect_jobs(&key(), &p, &[job(10, PipelineStatus::Failed)]);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].kind, TransitionKind::Failed);

        // Unchanged status emits nothing.
        let again = s.detect_jobs(&key(), &p, &[job(10, PipelineStatus::Failed)]);
        assert!(again.is_empty());
    }

    #[test]
    fn enabling_job_level_midsession_baselines_not_floods() {
        // A project can already be pipeline-tracked when the user turns the job-level toggle on.
        // The first job observation must still baseline, NOT emit a "started" for every job that
        // happened to be running at that moment.
        let mut s = PollState::default();
        let p = pipeline(1, PipelineStatus::Running);
        s.detect(&key(), std::slice::from_ref(&p)); // already pipeline-tracked (job-level was off)
        let out = s.detect_jobs(
            &key(),
            &p,
            &[
                job(10, PipelineStatus::Running),
                job(11, PipelineStatus::Running),
            ],
        );
        assert!(
            out.is_empty(),
            "enabling job-level mid-session must baseline, not flood"
        );
    }

    #[tokio::test]
    async fn poll_once_fetches_jobs_when_job_level_on() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 10, "status": "running", "ref": "main", "sha": "a", "web_url": "http://x/10", "updated_at": "t"}
            ])))
            .mount(&server)
            .await;
        // `.expect(1)` makes wiremock verify (on server drop) that the jobs endpoint WAS hit,
        // proving job-level polling actually fetches jobs for the newest pipeline.
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines/10/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 100, "name": "build", "status": "running", "stage": "build"}
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let tokens = MemoryTokenStore::new();
        tokens.store("acct", "tok").unwrap();
        let cfg = job_level_cfg(&server, true);
        let mut state = PollState::default();
        let first = poll_once(&mut state, &build_http_client(), &tokens, &cfg).await;
        assert!(first.is_empty(), "first poll baselines pipelines AND jobs");
    }

    #[tokio::test]
    async fn poll_once_skips_jobs_when_job_level_off() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 10, "status": "running", "ref": "main", "sha": "a", "web_url": "http://x/10", "updated_at": "t"}
            ])))
            .mount(&server)
            .await;
        // `.expect(0)`: when job-level is off, the jobs endpoint must NOT be called (bounded cost).
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines/10/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .expect(0)
            .mount(&server)
            .await;

        let tokens = MemoryTokenStore::new();
        tokens.store("acct", "tok").unwrap();
        let cfg = job_level_cfg(&server, false);
        let mut state = PollState::default();
        let _ = poll_once(&mut state, &build_http_client(), &tokens, &cfg).await;
    }

    /// One account, one monitored project (id 1), with `job_level` set as given.
    fn job_level_cfg(server: &MockServer, job_level: bool) -> Config {
        Config {
            accounts: vec![Account {
                id: "acct".into(),
                label: "l".into(),
                provider: crate::model::ProviderKind::Gitlab,
                base_url: server.uri(),
                identity: crate::model::Identity {
                    username: "u".into(),
                    name: None,
                    email: None,
                },
            }],
            monitored: vec![crate::model::MonitoredProject {
                account_id: "acct".into(),
                project_id: 1,
                name: "p1".into(),
                web_url: "u".into(),
                remote_ref: None,
            }],
            rules: crate::model::NotificationRules {
                on_start: true,
                on_success: true,
                on_fail: true,
                pipeline_level: true,
                job_level,
            },
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn poll_once_isolates_one_failing_project() {
        let server = MockServer::start().await;
        // Project 1 healthy; project 2 returns 500.
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 10, "status": "running", "ref": "main", "sha": "a", "web_url": "http://x/10", "updated_at": "t"}
            ])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/2/pipelines"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let tokens = MemoryTokenStore::new();
        tokens.store("acct", "tok").unwrap();
        let cfg = Config {
            accounts: vec![Account {
                id: "acct".into(),
                label: "l".into(),
                provider: crate::model::ProviderKind::Gitlab,
                base_url: server.uri(),
                identity: crate::model::Identity {
                    username: "u".into(),
                    name: None,
                    email: None,
                },
            }],
            monitored: vec![
                crate::model::MonitoredProject {
                    account_id: "acct".into(),
                    project_id: 1,
                    name: "p1".into(),
                    web_url: "u".into(),
                    remote_ref: None,
                },
                crate::model::MonitoredProject {
                    account_id: "acct".into(),
                    project_id: 2,
                    name: "p2".into(),
                    web_url: "u".into(),
                    remote_ref: None,
                },
            ],
            ..Config::default()
        };

        let mut state = PollState::default();
        // First poll baselines project 1 (no transitions); project 2 errors and is skipped.
        let first = poll_once(&mut state, &build_http_client(), &tokens, &cfg).await;
        assert!(first.is_empty());
        // Project 1's running pipeline is tracked despite project 2 failing.
        assert_eq!(state.aggregate_status(), Some(PipelineStatus::Running));
    }

    #[tokio::test]
    async fn poll_once_skips_keychain_for_account_without_monitored_projects() {
        // An account with no monitored projects must not trigger a keychain read. Otherwise the
        // poller asks for token access every tick, which on macOS pops a recurring authorization
        // prompt for an account that has nothing to poll.
        let tokens = MemoryTokenStore::new();
        tokens.store("acct", "tok").unwrap();
        let cfg = Config {
            accounts: vec![Account {
                id: "acct".into(),
                label: "l".into(),
                provider: crate::model::ProviderKind::Gitlab,
                base_url: "https://gitlab.example.com".into(),
                identity: crate::model::Identity {
                    username: "u".into(),
                    name: None,
                    email: None,
                },
            }],
            monitored: vec![], // nothing selected yet
            ..Config::default()
        };
        let mut state = PollState::default();
        let out = poll_once(&mut state, &build_http_client(), &tokens, &cfg).await;
        assert!(out.is_empty());
        assert_eq!(
            tokens.reads(),
            0,
            "keychain must not be read when no projects are monitored"
        );
    }
}
