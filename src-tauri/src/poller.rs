//! Background polling and pure transition detection.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::model::{Config, Job, Pipeline, PipelineStatus, MIN_POLL_SECS};
use crate::provider::{build_provider, Provider, ProviderError};
use crate::secrets::TokenStore;

/// `(account_id, project_id)` uniquely identifies a monitored project across accounts. A GitLab
/// project id is only unique within its instance/account, hence the account in the key.
pub type ProjectKey = (String, u64);

/// Minimum seconds between `token_health` checks per account. The check is the authoritative auth +
/// expiry probe, but running it every poll tick scales HTTP request volume with the poll cadence
/// (and on GitLab now costs two requests: `/user` + the PAT self endpoint). Throttling it to a
/// slower cadence keeps the per-tick cost to the project polls; between checks the last-known
/// auth/expiry state is retained.
const TOKEN_HEALTH_MIN_INTERVAL_SECS: i64 = 300;

/// A per-project status view for the popover panel rows. Built from [`PollState`] and handed to the
/// panel (via a command + a per-tick event) so each monitored project can show a colored indicator
/// plus its current branch, status word, and a relative "updated N ago" time.
///
/// `status` is `None` for a project that has only ever FAILED to poll (no pipeline observed yet):
/// combined with `stale = true`, the panel renders that as "can't connect". A project that has
/// polled successfully but has no current pipeline at all (no CI configured, or CI that has never
/// run) is `status: None` + `stale: false` + `no_pipelines: true`, distinct from a project that
/// simply has not been polled yet (absent from the snapshot entirely -> "checking").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectStatusView {
    /// Last-known normalized status, or `None` when this project has never polled successfully.
    pub status: Option<PipelineStatus>,
    pub branch: String,
    /// The latest pipeline's `updated_at` (RFC3339 from the provider); empty when never observed.
    /// The panel renders it as a relative time; an empty/unparseable value is simply not shown.
    pub updated_at: String,
    /// `true` when this project's most recent poll attempt FAILED. With a known `status` the row is
    /// shown as offline (last-known kept so a transient blip doesn't blank it); with `status: None`
    /// it has never succeeded, so the row reads "can't connect".
    pub stale: bool,
    /// `true` when this project HAS completed at least one successful poll but currently has no
    /// pipeline at all (no CI configured, or CI that has never run). Distinguishes a settled "no CI"
    /// row from the first-poll-still-in-flight case, which both otherwise carry `status: None` +
    /// `stale: false`.
    pub no_pipelines: bool,
    /// The current pipeline's own page (empty when there is none). `open_project_url` opens this
    /// instead of the project's static page while `status` is `Running`, so clicking an in-progress
    /// row lands on the active run rather than the repo's landing page.
    pub pipeline_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    Started,
    Succeeded,
    Failed,
    Canceled,
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

/// A per-account token-health event the poller emits for notification (parallel to [`Transition`]).
/// Deduped in-memory per run by [`PollState`], so each fires at most once per episode/bracket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenEventKind {
    /// The account's token is dead (expired / revoked / invalid) per `token_health`.
    AuthFailed,
    /// The token is valid but within a warning bracket; `hours` is the bracket ceiling (72 or 24).
    ExpiringSoon { hours: i64, expires_at: String },
    /// The OS credential store could not be reached to read a token (e.g. no Secret Service /
    /// GNOME Keyring / KWallet running on Linux). Distinct from a missing entry: this is the
    /// store itself being unavailable, which otherwise leaves the poller silently idle.
    KeychainUnavailable,
}

/// A token-health event for one account, carried to the notification layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenEvent {
    pub account_id: String,
    pub account_label: String,
    pub kind: TokenEventKind,
}

/// Per-account token health for the panel / accounts UI. Published each tick alongside the
/// per-project snapshot; runtime-only (never persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenHealthView {
    /// `true` when the most recent `token_health` check reported a dead token.
    pub auth_failed: bool,
    /// Last-known raw provider expiry string, or `None` (no expiry / not yet known / unavailable).
    pub expires_at: Option<String>,
}

/// Map a pipeline status to the transition kind worth notifying about (if any).
fn transition_for(status: PipelineStatus) -> Option<TransitionKind> {
    match status {
        PipelineStatus::Running => Some(TransitionKind::Started),
        PipelineStatus::Success => Some(TransitionKind::Succeeded),
        PipelineStatus::Failed => Some(TransitionKind::Failed),
        PipelineStatus::Canceled => Some(TransitionKind::Canceled),
        _ => None,
    }
}

/// The newest pipeline in a project's recent list: the one with the greatest id. Pipeline ids
/// increase with creation on both providers, so the greatest id is the most-recently-created run.
///
/// NOT the list head: GitLab lists by `updated_at desc`, so an older commit that settled most
/// recently (its completion bumped `updated_at`) can sort ahead of a newer commit that is still
/// building. Selecting by id keeps the aggregate anchored to the genuinely newest commit, so a
/// just-passed older pipeline can't mask a still-running newer one (the green-while-building bug).
fn newest_pipeline(latest: &[Pipeline]) -> Option<&Pipeline> {
    latest.iter().max_by_key(|p| p.id)
}

/// Build the representative pipeline whose status the project's aggregate reflects. A single push
/// fans out into multiple workflow runs sharing one `head_sha` (GitHub runs every matching
/// workflow); the newest commit is the run with the greatest id (see [`newest_pipeline`]), and its
/// `sha` names that commit's run group.
///
/// While ANY run of that commit is still in flight ([`is_in_flight`](PipelineStatus::is_in_flight),
/// i.e. running or queued) the commit as a whole is in flight: the worst in-flight status wins
/// (Running over Pending), so a sibling that has already failed never pre-empts a run that is still
/// going. Only once every run has settled does the worst terminal status win (Failed over a passed
/// or skipped run). This keeps the project "in progress" until all of that commit's runs settle,
/// then reflects the commit's true outcome.
///
/// This grouping only applies when the newest pipeline's provider genuinely fans one trigger out
/// into several simultaneous runs; see [`Pipeline::commit_fanout`] for why GitLab opts out and
/// returns the newest pipeline's own status unconditionally instead.
///
/// The newest run supplies the branch / url / timestamp; only its status is replaced by the
/// aggregate. Returns `None` for an empty list.
fn aggregate_current(latest: &[Pipeline]) -> Option<Pipeline> {
    let newest = newest_pipeline(latest)?;
    if !newest.commit_fanout {
        return Some(newest.clone());
    }
    let runs = || {
        latest
            .iter()
            .filter(|p| p.sha == newest.sha)
            .map(|p| p.status)
    };
    let status = runs()
        .filter(PipelineStatus::is_in_flight)
        .max_by_key(|s| s.severity())
        .or_else(|| runs().max_by_key(|s| s.severity()))
        .unwrap_or(newest.status);
    Some(Pipeline {
        status,
        ..newest.clone()
    })
}

/// Last-seen pipeline state, used to detect transitions across polls.
#[derive(Default)]
pub struct PollState {
    /// Last-seen status per pipeline id, per project.
    seen: HashMap<ProjectKey, HashMap<u64, PipelineStatus>>,
    /// Most recent pipeline per project. Its status ranks the aggregate tray icon; its branch
    /// (`ref_`) is shown on the project's tray row.
    current: HashMap<ProjectKey, Pipeline>,
    /// Last-seen status per job id, per project. Only the currently-tracked (newest) pipeline's
    /// jobs are kept (pruned each tick), so this stays bounded to one pipeline's worth per project.
    seen_jobs: HashMap<ProjectKey, HashMap<u64, PipelineStatus>>,
    /// Projects whose most recent poll attempt FAILED. They keep their last-known `current` entry
    /// (resilient to transient blips and self-healing on the next good poll), but are flagged so
    /// the tray can mark the row stale instead of showing old data as fresh.
    stale: HashSet<ProjectKey>,
    /// Accounts whose `token_health` most recently reported a dead token (HTTP 401 / revoked).
    auth_failed: HashSet<String>,
    /// Last-known token expiry per account (raw provider string); `None` = no expiry / unknown.
    token_expiry: HashMap<String, Option<String>>,
    /// Accounts already notified about the CURRENT auth-failure episode (cleared on recovery), so
    /// the "authentication failed" notification fires once per episode, not every tick.
    notified_auth_failed: HashSet<String>,
    /// Expiry brackets already warned this run, per account: `(expiry the brackets were warned for,
    /// brackets warned)`. Keyed on the raw expiry so 72h/24h each fire at most once for a given
    /// token, while a REPLACED token (different expiry) re-arms both brackets rather than inheriting
    /// the prior token's suppression.
    warned_brackets: HashMap<String, (String, HashSet<i64>)>,
    /// Epoch seconds of the last `token_health` check per account, to throttle it below the poll
    /// cadence (see [`TOKEN_HEALTH_MIN_INTERVAL_SECS`]).
    token_health_checked_at: HashMap<String, i64>,
    /// Whether the "credential store unavailable" notification has already fired for the current
    /// outage. The store is process-global (not per-account), so this dedupes to one notification
    /// per episode; a later successful read clears it so a recurrence re-notifies.
    keychain_unavailable_notified: bool,
}

impl PollState {
    /// Diff the latest pipelines for a project against last-seen state, returning the
    /// transitions to notify about.
    ///
    /// The FIRST observation of a project (including a project newly added to the monitored set
    /// while the poller is already running) seeds state and returns nothing, so pipelines that
    /// predate monitoring never produce notifications.
    ///
    /// `latest` carries the project's recent pipelines in no guaranteed order (GitLab lists by
    /// `updated_at` desc, GitHub by `created_at` desc). The newest commit is identified by greatest
    /// pipeline id, not list position (see [`newest_pipeline`]). The project's aggregate status
    /// spans ALL of that commit's runs (see [`aggregate_current`]), so a GitHub push that fans into
    /// several workflow runs stays Running until every run settles.
    pub fn detect(&mut self, key: &ProjectKey, latest: &[Pipeline]) -> Vec<Transition> {
        // Drive this project's aggregate status from every run of the newest commit, not just the
        // single newest run; an empty list means no current pipeline, so drop any stale entry.
        match aggregate_current(latest) {
            Some(rep) => {
                self.current.insert(key.clone(), rep);
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
        self.current
            .values()
            .map(|p| p.status)
            .max_by_key(|s| s.severity())
    }

    /// Snapshot of each tracked project's status, for the panel rows. Keyed by the same
    /// [`ProjectKey`] the monitored set joins on, so the panel can look each project up.
    ///
    /// Includes both projects that have polled successfully (carrying their last-known status, and
    /// flagged stale if the latest attempt failed) AND projects that have ONLY ever failed to poll
    /// (no pipeline yet): the latter are surfaced with `status: None` + `stale: true` so the panel
    /// shows "can't connect" instead of an indefinite "checking". A project never polled at all is
    /// absent from the map (the genuine first-poll-in-flight case).
    pub fn project_statuses(&self) -> HashMap<ProjectKey, ProjectStatusView> {
        let mut out: HashMap<ProjectKey, ProjectStatusView> = self
            .current
            .iter()
            .map(|(k, p)| {
                (
                    k.clone(),
                    ProjectStatusView {
                        status: Some(p.status),
                        branch: p.ref_.clone(),
                        updated_at: p.updated_at.clone(),
                        stale: self.stale.contains(k),
                        no_pipelines: false,
                        pipeline_url: p.web_url.clone(),
                    },
                )
            })
            .collect();
        // Projects that have only ever failed (in `stale`, no `current` entry) become unreachable
        // rows rather than being dropped. `or_insert_with` leaves the live entries above untouched.
        for k in &self.stale {
            out.entry(k.clone()).or_insert_with(|| ProjectStatusView {
                status: None,
                branch: String::new(),
                updated_at: String::new(),
                stale: true,
                no_pipelines: false,
                pipeline_url: String::new(),
            });
        }
        // Projects that HAVE completed at least one successful poll (tracked in `seen`, populated
        // by `detect` regardless of pipeline count) but currently have no pipeline at all settle
        // here rather than being dropped: absence from the map means "first poll still in flight",
        // which a repo with no CI configured (or CI that has never run) is not.
        for k in self.seen.keys() {
            out.entry(k.clone()).or_insert_with(|| ProjectStatusView {
                status: None,
                branch: String::new(),
                updated_at: String::new(),
                stale: false,
                no_pipelines: true,
                pipeline_url: String::new(),
            });
        }
        out
    }

    /// Flag a project as stale (its most recent poll attempt failed). Its `current` entry is kept.
    fn mark_stale(&mut self, key: &ProjectKey) {
        self.stale.insert(key.clone());
    }

    /// Clear a project's stale flag after a successful poll.
    fn mark_fresh(&mut self, key: &ProjectKey) {
        self.stale.remove(key);
    }

    /// Decide whether to emit an expiry warning for `account`, given the provider's raw `expires_at`
    /// and the current `now` (epoch secs), updating the per-run warned-bracket dedup. Returns the
    /// bracket (72 or 24) to warn at, or `None` (unparseable, outside all windows, or already warned).
    fn expiry_warning(&mut self, account: &str, expires_at: &str, now: i64) -> Option<i64> {
        let secs = crate::expiry::parse_expiry(expires_at)?;
        let hours = crate::expiry::hours_until(secs, now);
        let bracket = crate::expiry::current_bracket(hours)?;
        let entry = self
            .warned_brackets
            .entry(account.to_string())
            .or_insert_with(|| (expires_at.to_string(), HashSet::new()));
        // A changed expiry means a new token (in-place re-entry or provider rotation): forget the
        // prior token's warned brackets so the replacement gets its own 72h/24h warnings.
        if entry.0 != expires_at {
            entry.0 = expires_at.to_string();
            entry.1.clear();
        }
        let warned = &mut entry.1;
        if !warned.insert(bracket) {
            return None; // this bracket already warned for this token
        }
        // Also record any LARGER bracket, so entering the 24h window directly never later
        // re-fires a 72h warning for the same token.
        for &t in crate::expiry::THRESHOLDS_HOURS.iter() {
            if t >= bracket {
                warned.insert(t);
            }
        }
        Some(bracket)
    }

    /// Per-account token health for the panel / accounts UI: auth-failure flag + last-known expiry.
    pub fn token_health_snapshot(&self) -> HashMap<String, TokenHealthView> {
        let mut out: HashMap<String, TokenHealthView> = self
            .token_expiry
            .iter()
            .map(|(acct, expires_at)| {
                (
                    acct.clone(),
                    TokenHealthView {
                        auth_failed: self.auth_failed.contains(acct),
                        expires_at: expires_at.clone(),
                    },
                )
            })
            .collect();
        // An account that failed auth before any expiry was recorded still needs a row.
        for acct in &self.auth_failed {
            out.entry(acct.clone()).or_insert(TokenHealthView {
                auth_failed: true,
                expires_at: None,
            });
        }
        out
    }

    /// Drop tracked state for keys no longer in the monitored set, so an un-monitored project
    /// stops driving the aggregate and its transition history is forgotten.
    pub fn retain(&mut self, valid: &HashSet<ProjectKey>) {
        self.seen.retain(|k, _| valid.contains(k));
        self.current.retain(|k, _| valid.contains(k));
        self.seen_jobs.retain(|k, _| valid.contains(k));
        self.stale.retain(|k| valid.contains(k));
        // Token state is account-keyed; keep only accounts that still have a monitored project.
        let accounts: HashSet<&String> = valid.iter().map(|(a, _)| a).collect();
        self.auth_failed.retain(|a| accounts.contains(a));
        self.token_expiry.retain(|a, _| accounts.contains(a));
        self.notified_auth_failed.retain(|a| accounts.contains(a));
        self.warned_brackets.retain(|a, _| accounts.contains(a));
        self.token_health_checked_at
            .retain(|a, _| accounts.contains(a));
    }
}

/// Poll every monitored project once, returning all detected transitions. A fetch error for one
/// project is isolated: it is skipped this tick and does not affect other projects or the loop.
pub async fn poll_once(
    state: &mut PollState,
    http: &reqwest::Client,
    tokens: &dyn TokenStore,
    cfg: &Config,
    now: i64,
) -> (Vec<Transition>, Vec<TokenEvent>) {
    let mut transitions = Vec::new();
    let mut token_events = Vec::new();
    for acct in &cfg.accounts {
        // Nothing to poll for this account: skip it WITHOUT reading the keychain. Otherwise we
        // would request token access every tick for an account with no monitored projects, which
        // on macOS triggers a recurring keychain authorization prompt for no benefit.
        if !cfg.monitored.iter().any(|m| m.account_id == acct.id) {
            continue;
        }
        let token = match tokens.get(&acct.id) {
            Ok(Some(t)) => {
                // A successful read clears any prior credential-store outage so a recurrence
                // notifies again.
                state.keychain_unavailable_notified = false;
                t
            }
            // No token stored for this account is not a store failure: skip quietly.
            Ok(None) => {
                state.keychain_unavailable_notified = false;
                continue;
            }
            // The credential store itself is unreachable (e.g. no Secret Service on Linux). Without
            // a signal the poller would idle forever; emit one notification per outage, then skip.
            Err(_) => {
                if !state.keychain_unavailable_notified {
                    state.keychain_unavailable_notified = true;
                    let label = if acct.label.is_empty() {
                        acct.identity.username.clone()
                    } else {
                        acct.label.clone()
                    };
                    token_events.push(TokenEvent {
                        account_id: acct.id.clone(),
                        account_label: label,
                        kind: TokenEventKind::KeychainUnavailable,
                    });
                }
                continue;
            }
        };
        let provider = build_provider(
            acct.provider,
            http.clone(),
            acct.base_url.clone(),
            token.expose().to_string(),
        );

        // A display name for token-health notifications: the user's label, or the resolved username
        // when no label was set (mirrors the panel's account-name fallback). Built lazily so a tick
        // that emits no token event allocates nothing.
        let make_label = || {
            if acct.label.is_empty() {
                acct.identity.username.clone()
            } else {
                acct.label.clone()
            }
        };

        // Token health: the authoritative auth + expiry signal, distinct from the per-project
        // polling path below. Throttled to at most once per TOKEN_HEALTH_MIN_INTERVAL_SECS per
        // account so it does not fire an HTTP request on every tick; between checks the prior
        // auth/expiry state is retained.
        let due_for_health = match state.token_health_checked_at.get(&acct.id) {
            Some(&last) => now - last >= TOKEN_HEALTH_MIN_INTERVAL_SECS,
            None => true,
        };

        if due_for_health {
            state.token_health_checked_at.insert(acct.id.clone(), now);
            match provider.token_health().await {
                Err(ProviderError::Unauthorized) => {
                    // Dead token: flag the account, mark all its projects stale, SKIP polling them
                    // this tick (they would only 401 too), and notify once per failure episode.
                    state.auth_failed.insert(acct.id.clone());
                    state.token_expiry.insert(acct.id.clone(), None);
                    for mp in cfg.monitored.iter().filter(|m| m.account_id == acct.id) {
                        state.mark_stale(&(acct.id.clone(), mp.project_id));
                    }
                    if state.notified_auth_failed.insert(acct.id.clone()) {
                        token_events.push(TokenEvent {
                            account_id: acct.id.clone(),
                            account_label: make_label(),
                            kind: TokenEventKind::AuthFailed,
                        });
                    }
                    continue;
                }
                Ok(health) => {
                    // Healthy: clear any auth-failure (re-arms a future failure) and record expiry.
                    state.auth_failed.remove(&acct.id);
                    state.notified_auth_failed.remove(&acct.id);
                    state
                        .token_expiry
                        .insert(acct.id.clone(), health.expires_at.clone());
                    if let Some(exp) = &health.expires_at {
                        if let Some(bracket) = state.expiry_warning(&acct.id, exp, now) {
                            token_events.push(TokenEvent {
                                account_id: acct.id.clone(),
                                account_label: make_label(),
                                kind: TokenEventKind::ExpiringSoon {
                                    hours: bracket,
                                    expires_at: exp.clone(),
                                },
                            });
                        }
                    }
                }
                Err(_) => {
                    // Transient (rate-limit / 5xx / network): leave prior auth + expiry state intact
                    // and fall through to poll projects best-effort.
                }
            }
        } else if state.auth_failed.contains(&acct.id) {
            // Not yet due for a re-check and already known dead: skip polling its projects (they
            // would only 401), mirroring the dead path above but without re-notifying.
            for mp in cfg.monitored.iter().filter(|m| m.account_id == acct.id) {
                state.mark_stale(&(acct.id.clone(), mp.project_id));
            }
            continue;
        }

        for mp in cfg.monitored.iter().filter(|m| m.account_id == acct.id) {
            let key = (acct.id.clone(), mp.project_id);
            let pipelines = match provider
                .list_pipelines(mp.project_id, mp.remote_ref.as_deref())
                .await
            {
                Ok(p) => p,
                Err(_) => {
                    // Error isolation: skip this project this tick. Flag it stale so the tray
                    // marks its retained last-known status as no longer fresh, instead of
                    // showing it as current.
                    state.mark_stale(&key);
                    continue;
                }
            };
            // A successful fetch: this project's status is fresh again.
            state.mark_fresh(&key);
            let mut detected = state.detect(&key, &pipelines);

            // Job-level: fetch jobs only for the newest commit's pipeline (greatest id, not list
            // head, since GitLab orders by updated_at), so cost is at most one extra request per
            // project per tick and the jobs track the same pipeline the aggregate reflects. A
            // job-fetch error is isolated (skip job detection this tick) like a pipeline-fetch error.
            if cfg.rules.any_job_enabled() {
                if let Some(newest) = newest_pipeline(&pipelines) {
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
    (transitions, token_events)
}

/// Run the polling loop until the task is dropped. Each tick reads the current config (so
/// changes to the monitored set / interval take effect), polls once, and forwards transitions
/// and the new aggregate status to the callbacks. Spawned during app setup (Task 11).
pub async fn run_poller<F, G, H>(
    http: reqwest::Client,
    tokens: Arc<dyn TokenStore>,
    config: Arc<Mutex<Config>>,
    mut on_transitions: F,
    mut on_aggregate: G,
    mut on_token_events: H,
) where
    F: FnMut(&[Transition]),
    G: FnMut(
        Option<PipelineStatus>,
        &HashMap<ProjectKey, ProjectStatusView>,
        &HashMap<String, TokenHealthView>,
    ),
    H: FnMut(&[TokenEvent]),
{
    let mut state = PollState::default();
    loop {
        let cfg = config.lock().unwrap().clone();
        // Defensive floor in case an invalid value ever slips past validation.
        let interval_secs = cfg.poll_interval_secs.max(MIN_POLL_SECS);
        let (transitions, token_events) =
            poll_once(&mut state, &http, &*tokens, &cfg, crate::expiry::now_unix()).await;
        if !transitions.is_empty() {
            on_transitions(&transitions);
        }
        if !token_events.is_empty() {
            on_token_events(&token_events);
        }
        on_aggregate(
            state.aggregate_status(),
            &state.project_statuses(),
            &state.token_health_snapshot(),
        );
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
        // Repo metadata fetch: list_pipelines reads the default branch here, then scopes the runs
        // query to it.
        Mock::given(method("GET"))
            .and(path("/api/v3/repos/acme/web-app"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 1, "name": "web-app", "full_name": "acme/web-app",
                "html_url": "http://x/repo", "owner": {"login": "acme"}, "default_branch": "main"
            })))
            .mount(&server)
            .await;
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
        let (first, _) = poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_000).await;
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
            commit_fanout: true,
        }
    }

    fn job(id: u64, status: PipelineStatus) -> Job {
        Job {
            id,
            name: format!("job{id}"),
            status,
            stage: "test".into(),
            web_url: format!("http://x/job/{id}"),
        }
    }

    fn key() -> ProjectKey {
        ("acct".into(), 1)
    }

    /// Mount the GitLab single-project metadata endpoint so `list_pipelines` can read the default
    /// branch before fetching pipelines. Without it the project fetch 404s and the poll fails.
    async fn mount_gl_project(server: &MockServer, id: u64, default_branch: &str) {
        Mock::given(method("GET"))
            .and(path(format!("/api/v4/projects/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": id, "name": "p", "web_url": "u", "default_branch": default_branch
            })))
            .mount(server)
            .await;
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
    fn status_change_to_canceled_emits_canceled() {
        let mut s = PollState::default();
        s.detect(&key(), &[pipeline(1, PipelineStatus::Running)]); // baseline
        let out = s.detect(&key(), &[pipeline(1, PipelineStatus::Canceled)]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, TransitionKind::Canceled);
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
    fn aggregate_status_spans_newest_commits_runs_only() {
        // A GitHub push fans out into several workflow runs sharing one head_sha; the project's
        // status is the WORST across THAT commit's runs. A finished CI run (Success) must not mask
        // a still-running run (Running) for the same commit, and an OLDER commit's failed run, now
        // superseded, must not keep the project red either.
        let mut s = PollState::default();
        let mut ci = pipeline(3, PipelineStatus::Success);
        ci.sha = "newsha".into();
        let mut audit = pipeline(2, PipelineStatus::Running);
        audit.sha = "newsha".into();
        let mut older = pipeline(1, PipelineStatus::Failed);
        older.sha = "oldsha".into();
        // Newest-first: both runs of the new commit lead; the superseded old failure trails.
        s.detect(&key(), &[ci, audit, older]);
        assert_eq!(
            s.aggregate_status(),
            Some(PipelineStatus::Running),
            "the newest commit's still-running run wins; the older commit's failure is ignored"
        );
    }

    #[test]
    fn aggregate_keeps_commit_in_flight_until_every_run_settles() {
        // Within one commit's runs, an in-flight run (Running/Pending) outranks a settled failure:
        // a CI run that has already failed must NOT flip the project to Failed while a sibling run
        // (e.g. Security audit) is still running. Only once all runs settle does the worst terminal
        // status win. pipeline() gives every run the same sha, so these are one commit's runs.
        let mut s = PollState::default();
        s.detect(
            &key(),
            &[
                pipeline(3, PipelineStatus::Running),
                pipeline(2, PipelineStatus::Failed),
            ],
        );
        assert_eq!(
            s.aggregate_status(),
            Some(PipelineStatus::Running),
            "a still-running sibling keeps the commit Running despite a failed run"
        );

        // All runs now settled: the worst terminal status (Failed) wins.
        s.detect(
            &key(),
            &[
                pipeline(3, PipelineStatus::Success),
                pipeline(2, PipelineStatus::Failed),
            ],
        );
        assert_eq!(
            s.aggregate_status(),
            Some(PipelineStatus::Failed),
            "once every run settles, the worst terminal status wins"
        );
    }

    #[test]
    fn aggregate_follows_newest_commit_not_most_recently_updated() {
        // Real-world overlap: two pipelines on the default branch for DIFFERENT commits. An older
        // commit's pipeline finishes Success AFTER a newer commit's pipeline has started, so the
        // older one carries the LATER `updated_at`. GitLab lists by `updated_at desc`, so the older,
        // settled pipeline sorts FIRST even though the newer commit's pipeline is still running.
        // Picking the list head would let the settled older run mask the in-flight newer one (project
        // shows green while the newer commit builds). The aggregate must follow the newest commit by id.
        let mut newer_running = pipeline(12, PipelineStatus::Running);
        newer_running.sha = "commitb".into();
        newer_running.updated_at = "2026-01-01T10:00:00Z".into();
        let mut older_success = pipeline(11, PipelineStatus::Success);
        older_success.sha = "commita".into();
        older_success.updated_at = "2026-01-01T10:05:00Z".into(); // settled later -> sorts first
        let mut s = PollState::default();
        // Order as the provider delivers it: most-recently-updated (the settled older commit) first.
        s.detect(&key(), &[older_success, newer_running]);
        assert_eq!(
            s.aggregate_status(),
            Some(PipelineStatus::Running),
            "a still-building newer commit must not be masked by an older commit that settled later"
        );
    }

    #[test]
    fn gitlab_stale_scheduled_pipeline_does_not_redden_a_later_pipeline_on_the_same_sha() {
        // GitLab's scheduled (and api-triggered) pipelines run against the branch's CURRENT head sha,
        // not a new commit, so two pipelines can share a sha while being unrelated, time-separated
        // triggers: an old nightly schedule that failed, and a later pipeline that passed against the
        // very same still-unchanged commit. GitLab pipelines are not a push fan-out (commit_fanout =
        // false), so the newer pipeline's own status must win outright -- the older scheduled failure
        // must not redden it.
        let mut newer = pipeline(2, PipelineStatus::Success);
        newer.sha = "sharedsha".into();
        newer.commit_fanout = false;
        let mut older = pipeline(1, PipelineStatus::Failed);
        older.sha = "sharedsha".into();
        older.commit_fanout = false;
        let mut s = PollState::default();
        s.detect(&key(), &[newer, older]);
        assert_eq!(
            s.aggregate_status(),
            Some(PipelineStatus::Success),
            "a stale scheduled failure on an unchanged sha must not redden a later, unrelated pass"
        );
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
    fn polled_project_with_no_pipelines_is_distinct_from_never_polled() {
        // A repo with no CI configured (or whose CI has never run) polls successfully every tick
        // but always gets an empty pipeline list. That must NOT look identical to "first poll
        // still in flight" -- both would otherwise render as an indefinite "checking" row.
        let mut s = PollState::default();
        s.detect(&key(), &[]); // first (and only) observation: poll succeeded, zero pipelines
        let view = s.project_statuses().remove(&key()).expect(
            "a successfully-polled project must appear in the snapshot even with no pipelines",
        );
        assert_eq!(view.status, None);
        assert!(!view.stale);
        assert!(view.no_pipelines);

        // A project that has never been polled at all stays absent from the snapshot.
        let never_polled: ProjectKey = ("acct".into(), 99);
        assert!(!s.project_statuses().contains_key(&never_polled));
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
        mount_gl_project(&server, 1, "main").await;
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
                {"id": 100, "name": "build", "status": "running", "stage": "build",
                 "web_url": "http://x/p/10/jobs/100"}
            ])))
            .expect(1)
            .mount(&server)
            .await;

        let tokens = MemoryTokenStore::new();
        tokens.store("acct", "tok").unwrap();
        let cfg = job_level_cfg(&server, true);
        let mut state = PollState::default();
        let (first, _) = poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_000).await;
        assert!(first.is_empty(), "first poll baselines pipelines AND jobs");
    }

    #[tokio::test]
    async fn poll_once_skips_jobs_when_job_level_off() {
        let server = MockServer::start().await;
        mount_gl_project(&server, 1, "main").await;
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
        let _ = poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_000).await;
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
                on_cancel: true,
                job_on_start: job_level,
                job_on_success: job_level,
                job_on_fail: job_level,
                job_on_cancel: job_level,
            },
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn poll_once_isolates_one_failing_project() {
        let server = MockServer::start().await;
        // Project 1 healthy; project 2 returns 500 (here, on its metadata fetch).
        mount_gl_project(&server, 1, "main").await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 10, "status": "running", "ref": "main", "sha": "a", "web_url": "http://x/10", "updated_at": "t"}
            ])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/2"))
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
        let (first, _) = poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_000).await;
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
        let (out, _) = poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_000).await;
        assert!(out.is_empty());
        assert_eq!(
            tokens.reads(),
            0,
            "keychain must not be read when no projects are monitored"
        );
    }

    #[test]
    fn project_statuses_reports_status_and_branch_per_project() {
        let mut s = PollState::default();
        let mut p_a = pipeline(1, PipelineStatus::Running);
        p_a.ref_ = "main".into();
        let mut p_b = pipeline(2, PipelineStatus::Failed);
        p_b.ref_ = "develop".into();
        s.detect(&("acct".into(), 10), &[p_a]);
        s.detect(&("acct".into(), 20), &[p_b]);

        let snap = s.project_statuses();
        let a = snap
            .get(&("acct".to_string(), 10))
            .expect("project a present in snapshot");
        assert_eq!(a.status, Some(PipelineStatus::Running));
        assert_eq!(a.branch, "main");
        assert_eq!(
            a.updated_at, "2026-06-20T00:00:00Z",
            "the latest pipeline's updated_at is surfaced for the relative-time row"
        );
        assert_eq!(
            a.pipeline_url, "http://x/1",
            "the current pipeline's own page is surfaced for the open-on-click command"
        );
        let b = snap
            .get(&("acct".to_string(), 20))
            .expect("project b present in snapshot");
        assert_eq!(b.status, Some(PipelineStatus::Failed));
        assert_eq!(b.branch, "develop", "branch is tracked per project");
    }

    #[tokio::test]
    async fn never_succeeded_failing_project_is_unreachable() {
        // A project whose FIRST (and only) poll fails has no last-known pipeline. It must still be
        // surfaced (status None + stale true) so the panel reads "can't connect" rather than an
        // indefinite "checking" (which would mean a poll is genuinely still in flight).
        let tokens = MemoryTokenStore::new();
        tokens.store("acct", "tok").unwrap();
        let key = ("acct".to_string(), 1u64);
        let mut state = PollState::default();

        // The only poll hits a dead address: the project never enters `current`.
        let cfg_dead = stale_cfg("http://127.0.0.1:1");
        poll_once(&mut state, &build_http_client(), &tokens, &cfg_dead, 1_000).await;

        let snap = state.project_statuses();
        let v = snap
            .get(&key)
            .expect("a never-succeeded but failing project is surfaced, not dropped");
        assert_eq!(v.status, None, "no pipeline has ever been observed");
        assert!(v.stale, "its failing poll marks it unreachable");
        assert!(v.branch.is_empty(), "no last-known branch to show");
    }

    /// One gitlab account + one monitored project (id 1) pointed at `base`.
    fn stale_cfg(base: &str) -> Config {
        Config {
            accounts: vec![Account {
                id: "acct".into(),
                label: "l".into(),
                provider: crate::model::ProviderKind::Gitlab,
                base_url: base.into(),
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
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn poll_failure_marks_project_stale_keeping_last_known() {
        let server = MockServer::start().await;
        mount_gl_project(&server, 1, "main").await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 10, "status": "success", "ref": "main", "sha": "a", "web_url": "http://x/10", "updated_at": "t"}
            ])))
            .mount(&server)
            .await;
        let tokens = MemoryTokenStore::new();
        tokens.store("acct", "tok").unwrap();
        let key = ("acct".to_string(), 1u64);

        let mut state = PollState::default();
        // First poll against the live server: seeds last-known status/branch, fresh (not stale).
        let cfg_live = stale_cfg(&server.uri());
        poll_once(&mut state, &build_http_client(), &tokens, &cfg_live, 1_000).await;
        let fresh = state.project_statuses();
        let v = fresh.get(&key).expect("present after a good poll");
        assert_eq!(v.status, Some(PipelineStatus::Success));
        assert_eq!(v.branch, "main");
        assert!(!v.stale, "fresh after a successful poll");

        // Second poll against a dead address: the fetch errors, so the project is marked stale
        // but keeps its last-known status and branch (no flicker to unknown).
        let cfg_dead = stale_cfg("http://127.0.0.1:1");
        poll_once(&mut state, &build_http_client(), &tokens, &cfg_dead, 1_000).await;
        let after = state.project_statuses();
        let v = after
            .get(&key)
            .expect("still present (last-known retained on failure)");
        assert_eq!(
            v.status,
            Some(PipelineStatus::Success),
            "last-known status retained"
        );
        assert_eq!(v.branch, "main", "last-known branch retained");
        assert!(v.stale, "a failed poll marks the project stale");

        // Third poll back against the live server clears the stale flag.
        poll_once(&mut state, &build_http_client(), &tokens, &cfg_live, 1_000).await;
        let recovered = state.project_statuses();
        assert!(
            !recovered.get(&key).unwrap().stale,
            "a successful poll clears stale"
        );
    }

    #[test]
    fn expiry_warning_brackets_and_dedups() {
        let mut s = PollState::default();
        // A FIXED expiry 72h after the epoch; `now` advances toward it (a real token's expiry string
        // never changes as it ages -- only `now` moves).
        let exp = "1970-01-04";
        // Entering the 72h window (71h remaining) fires the 72h bracket once, then dedups.
        assert_eq!(s.expiry_warning("a", exp, 3_600), Some(72));
        assert_eq!(s.expiry_warning("a", exp, 3_600), None);
        // The same token later drops into the 24h window (23h remaining): 24h fires once, then dedups.
        assert_eq!(s.expiry_warning("a", exp, 176_400), Some(24));
        assert_eq!(s.expiry_warning("a", exp, 176_400), None);
        // A token whose expiry is outside every window (1970-01-10 = 216h out) never warns.
        assert_eq!(s.expiry_warning("c", "1970-01-10", 0), None);
    }

    #[test]
    fn expiry_warning_direct_24h_suppresses_later_72h() {
        let mut s = PollState::default();
        let exp = "1970-01-04"; // fixed 72h-after-epoch expiry.
                                // First observed already inside the 24h window (23h remaining).
        assert_eq!(s.expiry_warning("b", exp, 176_400), Some(24));
        // A later reading of the SAME token back in the 72h range (e.g. a backward clock adjustment)
        // must NOT re-fire a 72h warning: the expiry is unchanged, so the dedup still holds.
        assert_eq!(s.expiry_warning("b", exp, 3_600), None);
    }

    #[test]
    fn expiry_warning_rearms_for_a_replaced_token() {
        let mut s = PollState::default();
        // Old token expiring in 24h: warned once at the 24h bracket (which also records 72h).
        assert_eq!(s.expiry_warning("a", "1970-01-02", 0), Some(24));
        assert_eq!(s.expiry_warning("a", "1970-01-02", 0), None);
        // The account's token is replaced (in-place re-entry or provider rotation) with one whose
        // expiry differs and sits in the 72h window. The new expiry must re-arm the dedup so the
        // replacement's 72h warning fires instead of being suppressed by the prior token's brackets.
        assert_eq!(s.expiry_warning("a", "1970-01-04", 0), Some(72));
    }

    #[test]
    fn token_health_snapshot_reports_auth_and_expiry() {
        let mut s = PollState::default();
        s.token_expiry.insert("a".into(), Some("2026-08-15".into()));
        s.token_expiry.insert("b".into(), None);
        s.auth_failed.insert("b".into());
        let snap = s.token_health_snapshot();
        assert_eq!(
            snap.get("a").unwrap().expires_at.as_deref(),
            Some("2026-08-15")
        );
        assert!(!snap.get("a").unwrap().auth_failed);
        assert!(snap.get("b").unwrap().auth_failed);
    }

    #[tokio::test]
    async fn token_health_unauthorized_flags_account_skips_polls_emits_once() {
        let server = MockServer::start().await;
        // token_health probes /user for liveness; a 401 there is the dead token (any token type).
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        // A dead account must NOT poll its projects this tick (they would only 401 too).
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .expect(0)
            .mount(&server)
            .await;

        let tokens = MemoryTokenStore::new();
        tokens.store("acct", "tok").unwrap();
        let cfg = job_level_cfg(&server, false);
        let mut state = PollState::default();

        let (_t, events) = poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_000).await;
        assert_eq!(events.len(), 1, "one AuthFailed event on first failure");
        assert!(matches!(events[0].kind, TokenEventKind::AuthFailed));
        assert_eq!(events[0].account_id, "acct");
        assert!(
            state
                .token_health_snapshot()
                .get("acct")
                .unwrap()
                .auth_failed,
            "account flagged auth_failed"
        );

        // Second tick past the health-check throttle window (so token_health genuinely re-runs):
        // still failing, but no repeat notification (deduped per episode).
        let (_t2, events2) =
            poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_400).await;
        assert!(events2.is_empty(), "auth-failure notifies once per episode");
    }

    /// A token store whose reads always fail, simulating an unavailable OS credential service
    /// (e.g. no Secret Service / GNOME Keyring / KWallet running on Linux).
    struct FailingTokenStore;
    impl TokenStore for FailingTokenStore {
        fn store(&self, _: &str, _: &str) -> Result<(), crate::secrets::TokenStoreError> {
            Ok(())
        }
        fn get(
            &self,
            _: &str,
        ) -> Result<Option<crate::secrets::SecretToken>, crate::secrets::TokenStoreError> {
            Err(crate::secrets::TokenStoreError(
                "service unavailable".into(),
            ))
        }
        fn delete(&self, _: &str) -> Result<(), crate::secrets::TokenStoreError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn keychain_read_failure_emits_unavailable_event_once() {
        // A read error from the credential store (not a missing entry) must surface a one-time
        // "keychain unavailable" event instead of silently skipping the account every tick.
        let server = MockServer::start().await;
        let cfg = job_level_cfg(&server, false);
        let tokens = FailingTokenStore;
        let mut state = PollState::default();

        let (_t, events) = poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_000).await;
        assert_eq!(
            events.len(),
            1,
            "one KeychainUnavailable event on the first failed read"
        );
        assert!(matches!(
            events[0].kind,
            TokenEventKind::KeychainUnavailable
        ));

        // Deduped per episode: a second tick with the store still failing does not re-notify.
        let (_t2, events2) =
            poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_400).await;
        assert!(
            events2.is_empty(),
            "keychain-unavailable notifies once per episode, not every tick"
        );
    }

    #[tokio::test]
    async fn token_health_throttled_to_one_request_per_interval() {
        // token_health must NOT fire an HTTP request every tick. Two ticks within the throttle
        // window perform the health check (here, the self-introspection request) at most once.
        let server = MockServer::start().await;
        mount_gl_project(&server, 1, "main").await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"username": "u"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;
        // `.expect(1)`: the throttled self endpoint is hit exactly once across the two ticks.
        Mock::given(method("GET"))
            .and(path("/api/v4/personal_access_tokens/self"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 1, "active": true, "revoked": false, "expires_at": null
            })))
            .expect(1)
            .mount(&server)
            .await;

        let tokens = MemoryTokenStore::new();
        tokens.store("acct", "tok").unwrap();
        let cfg = job_level_cfg(&server, false);
        let mut state = PollState::default();
        // Two ticks 60s apart -> well inside the 300s window -> token_health runs only on the first.
        poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_000).await;
        poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_060).await;
        // wiremock verifies `.expect(1)` when `server` drops at end of scope.
    }

    #[tokio::test]
    async fn token_health_recovery_clears_auth_failed() {
        let server = MockServer::start().await;
        mount_gl_project(&server, 1, "main").await;
        // Liveness passes (/user 200), then expiry is read from the self endpoint.
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "username": "u"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/personal_access_tokens/self"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 1, "active": true, "revoked": false, "expires_at": null
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/1/pipelines"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 10, "status": "success", "ref": "main", "sha": "a", "web_url": "http://x/10", "updated_at": "t"}
            ])))
            .mount(&server)
            .await;

        let tokens = MemoryTokenStore::new();
        tokens.store("acct", "tok").unwrap();
        let cfg = job_level_cfg(&server, false);
        let mut state = PollState::default();
        // A prior tick had flagged this account dead.
        state.auth_failed.insert("acct".to_string());
        state.notified_auth_failed.insert("acct".to_string());

        let _ = poll_once(&mut state, &build_http_client(), &tokens, &cfg, 1_000).await;
        assert!(
            !state
                .token_health_snapshot()
                .get("acct")
                .unwrap()
                .auth_failed,
            "a healthy check clears auth_failed"
        );
        assert!(
            !state.notified_auth_failed.contains("acct"),
            "the account is re-armed for a future failure"
        );
    }
}
