//! GitHub implementation of the [`Provider`] trait.
//!
//! Talks to the GitHub Actions REST API. github.com's API host is `api.github.com`; GitHub
//! Enterprise Server's API is at `<base_url>/api/v3`. Requests carry a `Bearer` token plus the
//! `Accept`/`X-GitHub-Api-Version` headers GitHub expects. The base URL is validated at the
//! command layer and the shared client disables cross-host redirects, so a token is only ever
//! sent to the validated host.

use serde::Deserialize;

use crate::model::{Identity, Job, Pipeline, PipelineStatus};
use crate::provider::{DiscoveredProject, Provider, ProviderError, TokenHealth};
use crate::secrets::SecretToken;

const PER_PAGE: u32 = 100;
/// Hard ceiling on pages fetched, so a misbehaving server cannot drive an unbounded loop.
const MAX_PAGES: u32 = 1000;
const API_VERSION: &str = "2022-11-28";

/// A GitHub provider bound to one account's base URL and token.
pub struct GithubProvider {
    client: reqwest::Client,
    /// Instance base URL, without a trailing slash (e.g. `https://github.com`, or a GHE host).
    base_url: String,
    /// Held as a `SecretToken` so the copy zeroizes on drop and never appears in Debug.
    token: SecretToken,
}

impl GithubProvider {
    pub fn new(
        client: reqwest::Client,
        base_url: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        GithubProvider {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: SecretToken::new(token),
        }
    }

    /// Build a full request URL. github.com (and a directly-entered `api.github.com`) map to the
    /// `api.github.com` host; any other host is treated as GitHub Enterprise Server, whose API
    /// lives under `<base_url>/api/v3`.
    fn url(&self, path_and_query: &str) -> String {
        let host = url::Url::parse(&self.base_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        if host == "github.com" || host == "api.github.com" {
            format!("https://api.github.com{path_and_query}")
        } else {
            format!("{}/api/v3{path_and_query}", self.base_url)
        }
    }

    async fn get(&self, path_and_query: &str) -> Result<reqwest::Response, ProviderError> {
        self.client
            .get(self.url(path_and_query))
            .header("Authorization", format!("Bearer {}", self.token.expose()))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .send()
            .await
            // `without_url` keeps the (token-free) URL out of the message regardless.
            .map_err(|e| ProviderError::Network(e.without_url().to_string()))
    }
}

/// Map an HTTP status code to a provider error, or `None` for 2xx.
fn classify(code: u16) -> Option<ProviderError> {
    match code {
        c if (200..300).contains(&c) => None,
        // GitHub also returns 403 for rate limiting; treated as Unauthorized here (see plan SHORTCUT).
        401 | 403 => Some(ProviderError::Unauthorized),
        c => Some(ProviderError::Http(c)),
    }
}

fn redact_json_err(e: reqwest::Error) -> ProviderError {
    ProviderError::Network(e.without_url().to_string())
}

#[derive(Deserialize)]
struct GhUser {
    login: String,
    name: Option<String>,
    email: Option<String>,
}

#[derive(Deserialize)]
struct GhOwner {
    login: String,
}

#[derive(Deserialize)]
struct GhRepo {
    id: u64,
    name: String,
    full_name: String,
    html_url: String,
    owner: GhOwner,
    /// The repo's default branch. Present on the single-repo endpoint; absent on the lighter
    /// project-discovery listing, hence `Option`.
    default_branch: Option<String>,
}

#[derive(Deserialize)]
struct GhRunsResponse {
    workflow_runs: Vec<GhRun>,
}

#[derive(Deserialize)]
struct GhRun {
    id: u64,
    head_branch: Option<String>,
    head_sha: Option<String>,
    /// Triggering event. GitHub's managed workflows (e.g. the "Dependabot Updates" updater) report
    /// `dynamic`; real CI runs report `push`, `pull_request`, `schedule`, etc.
    event: Option<String>,
    status: String,
    conclusion: Option<String>,
    html_url: String,
    updated_at: Option<String>,
}

#[derive(Deserialize)]
struct GhJobsResponse {
    jobs: Vec<GhJob>,
}

#[derive(Deserialize)]
struct GhJob {
    id: u64,
    name: String,
    status: String,
    conclusion: Option<String>,
    html_url: String,
}

impl Provider for GithubProvider {
    async fn validate_token(&self) -> Result<Identity, ProviderError> {
        let resp = self.get("/user").await?;
        if let Some(err) = classify(resp.status().as_u16()) {
            return Err(err);
        }
        let u: GhUser = resp.json().await.map_err(redact_json_err)?;
        Ok(Identity {
            username: u.login,
            name: u.name,
            email: u.email,
        })
    }

    async fn token_health(&self) -> Result<TokenHealth, ProviderError> {
        let resp = self.get("/user").await?;
        // Do NOT use classify() here: it maps 403 -> Unauthorized, but a GitHub 403 is a
        // rate-limit / permission signal, NOT a dead token. Only 401 means the token is dead.
        match resp.status().as_u16() {
            200..=299 => {
                // GitHub returns the token's expiry on every authenticated response (when the
                // token has one). HeaderMap lookup is case-insensitive.
                let expires_at = resp
                    .headers()
                    .get("github-authentication-token-expiration")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                Ok(TokenHealth { expires_at })
            }
            401 => Err(ProviderError::Unauthorized),
            c => Err(ProviderError::Http(c)),
        }
    }

    async fn list_projects(&self) -> Result<Vec<DiscoveredProject>, ProviderError> {
        let mut out = Vec::new();
        let mut page: u32 = 1;
        let mut pages_fetched: u32 = 0;
        loop {
            pages_fetched += 1;
            if pages_fetched > MAX_PAGES {
                break;
            }
            let resp = self
                .get(&format!(
                    "/user/repos?per_page={PER_PAGE}&sort=updated&page={page}"
                ))
                .await?;
            if let Some(err) = classify(resp.status().as_u16()) {
                return Err(err);
            }
            let repos: Vec<GhRepo> = resp.json().await.map_err(redact_json_err)?;
            let count = repos.len();
            out.extend(repos.into_iter().map(|r| DiscoveredProject {
                id: r.id,
                name: r.name,
                web_url: r.html_url,
                group: r.owner.login,
                remote_ref: Some(r.full_name),
            }));
            // GitHub paginates via the Link header; a short page is a reliable end-of-list signal.
            if count < PER_PAGE as usize {
                break;
            }
            page += 1;
        }
        Ok(out)
    }

    async fn list_pipelines(
        &self,
        project_id: u64,
        remote_ref: Option<&str>,
    ) -> Result<Vec<Pipeline>, ProviderError> {
        let repo = remote_ref
            .ok_or_else(|| ProviderError::Network("missing repository identifier".into()))?;

        // Scope the project's status to its default branch. A repo fans out into runs on many
        // branches (Dependabot PR branches, feature branches); the user monitors the repo's
        // mainline health, not whatever branch last ran. Fetch the repo to learn its default branch.
        let repo_resp = self.get(&format!("/repos/{repo}")).await?;
        if let Some(err) = classify(repo_resp.status().as_u16()) {
            return Err(err);
        }
        let meta: GhRepo = repo_resp.json().await.map_err(redact_json_err)?;
        let default_branch = meta.default_branch.unwrap_or_default();

        let resp = self
            .get(&format!(
                "/repos/{repo}/actions/runs?branch={default_branch}&per_page=20"
            ))
            .await?;
        if let Some(err) = classify(resp.status().as_u16()) {
            return Err(err);
        }
        let body: GhRunsResponse = resp.json().await.map_err(redact_json_err)?;
        Ok(body
            .workflow_runs
            .into_iter()
            // Drop GitHub's managed/bot runs (the "Dependabot Updates" updater and similar report
            // event=dynamic): they run on the default branch but are housekeeping, not the repo's
            // CI, so their failures must not redden the project.
            .filter(|r| r.event.as_deref() != Some("dynamic"))
            .map(|r| Pipeline {
                id: r.id,
                project_id,
                status: PipelineStatus::from_github(&r.status, r.conclusion.as_deref()),
                ref_: r.head_branch.unwrap_or_default(),
                sha: r.head_sha.unwrap_or_default(),
                web_url: r.html_url,
                updated_at: r.updated_at.unwrap_or_default(),
            })
            .collect())
    }

    async fn list_jobs(
        &self,
        _project_id: u64,
        remote_ref: Option<&str>,
        run_id: u64,
    ) -> Result<Vec<Job>, ProviderError> {
        let repo = remote_ref
            .ok_or_else(|| ProviderError::Network("missing repository identifier".into()))?;
        // SHORTCUT: single per_page=100 page (no pagination), mirroring GitLab's bound; upgrade to
        // Link-header pagination if a run with >100 jobs ever needs full coverage.
        let resp = self
            .get(&format!(
                "/repos/{repo}/actions/runs/{run_id}/jobs?per_page=100"
            ))
            .await?;
        if let Some(err) = classify(resp.status().as_u16()) {
            return Err(err);
        }
        let body: GhJobsResponse = resp.json().await.map_err(redact_json_err)?;
        Ok(body
            .jobs
            .into_iter()
            .map(|j| Job {
                id: j.id,
                name: j.name,
                status: PipelineStatus::from_github(&j.status, j.conclusion.as_deref()),
                // GitHub Actions has no "stage" concept (steps live inside jobs).
                stage: String::new(),
                web_url: j.html_url,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::build_http_client;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(server: &MockServer) -> GithubProvider {
        GithubProvider::new(build_http_client(), server.uri(), "test-token")
    }

    #[test]
    fn url_maps_github_com_and_ghe_hosts() {
        let c = build_http_client();
        // github.com -> api.github.com (no /api/v3).
        assert_eq!(
            GithubProvider::new(c.clone(), "https://github.com", "t").url("/user"),
            "https://api.github.com/user"
        );
        // A directly-entered api.github.com also maps to api.github.com (no doubled /api/v3).
        assert_eq!(
            GithubProvider::new(c.clone(), "https://api.github.com", "t").url("/user"),
            "https://api.github.com/user"
        );
        // Any other host is GitHub Enterprise Server: API under /api/v3.
        assert_eq!(
            GithubProvider::new(c, "https://ghe.corp.com", "t").url("/user"),
            "https://ghe.corp.com/api/v3/user"
        );
    }

    #[tokio::test]
    async fn validate_token_ok_returns_identity() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/user"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "login": "octocat", "name": "The Octocat", "email": "octo@example.com"
            })))
            .mount(&server)
            .await;

        let id = provider(&server).validate_token().await.unwrap();
        // GitHub's `login` maps onto our `username`.
        assert_eq!(id.username, "octocat");
        assert_eq!(id.name.as_deref(), Some("The Octocat"));
        assert_eq!(id.email.as_deref(), Some("octo@example.com"));
    }

    #[tokio::test]
    async fn validate_token_401_is_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = provider(&server).validate_token().await.unwrap_err();
        assert_eq!(err, ProviderError::Unauthorized);
    }

    #[tokio::test]
    async fn list_projects_paginates_and_maps_owner_and_remote_ref() {
        let server = MockServer::start().await;
        // Page 1 is full (100 entries) so a second page is fetched; page 2 is short, ending it.
        let page1: Vec<serde_json::Value> = (0..100)
            .map(|i| {
                serde_json::json!({
                    "id": i, "name": format!("repo{i}"), "full_name": format!("acme/repo{i}"),
                    "html_url": format!("https://github.com/acme/repo{i}"),
                    "owner": {"login": "acme"}
                })
            })
            .collect();
        Mock::given(method("GET"))
            .and(path("/api/v3/user/repos"))
            .and(query_param("page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page1))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v3/user/repos"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 999, "name": "dotfiles", "full_name": "octocat/dotfiles",
                 "html_url": "https://github.com/octocat/dotfiles", "owner": {"login": "octocat"}}
            ])))
            .mount(&server)
            .await;

        let projects = provider(&server).list_projects().await.unwrap();
        assert_eq!(projects.len(), 101, "both pages collected");
        assert_eq!(projects[0].group, "acme");
        assert_eq!(projects[0].remote_ref.as_deref(), Some("acme/repo0"));
        let last = projects.last().unwrap();
        assert_eq!(last.group, "octocat");
        assert_eq!(last.remote_ref.as_deref(), Some("octocat/dotfiles"));
    }

    #[tokio::test]
    async fn list_pipelines_scopes_to_default_branch_and_drops_managed_runs() {
        let server = MockServer::start().await;
        // The repo is fetched first to learn its default branch.
        Mock::given(method("GET"))
            .and(path("/api/v3/repos/acme/web-app"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 1, "name": "web-app", "full_name": "acme/web-app",
                "html_url": "https://github.com/acme/web-app",
                "owner": {"login": "acme"}, "default_branch": "main"
            })))
            .mount(&server)
            .await;
        // Runs are then requested filtered to that branch. A managed (event=dynamic) run on the
        // same branch must be dropped so housekeeping workflows cannot redden the project.
        Mock::given(method("GET"))
            .and(path("/api/v3/repos/acme/web-app/actions/runs"))
            .and(query_param("branch", "main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_count": 2,
                "workflow_runs": [
                    {"id": 55, "head_branch": "main", "head_sha": "abc123", "event": "push",
                     "status": "completed", "conclusion": "failure",
                     "html_url": "https://github.com/acme/web-app/actions/runs/55",
                     "updated_at": "2026-06-23T00:00:00Z"},
                    {"id": 56, "head_branch": "main", "head_sha": "abc123", "event": "dynamic",
                     "status": "completed", "conclusion": "failure",
                     "html_url": "https://github.com/acme/web-app/actions/runs/56",
                     "updated_at": "2026-06-23T00:00:00Z"}
                ]
            })))
            .mount(&server)
            .await;

        let pipes = provider(&server)
            .list_pipelines(7, Some("acme/web-app"))
            .await
            .unwrap();
        assert_eq!(pipes.len(), 1, "the managed event=dynamic run is dropped");
        assert_eq!(pipes[0].id, 55);
        assert_eq!(pipes[0].project_id, 7);
        assert_eq!(pipes[0].status, PipelineStatus::Failed);
        assert_eq!(pipes[0].ref_, "main");
        assert_eq!(pipes[0].sha, "abc123");
        assert_eq!(
            pipes[0].web_url,
            "https://github.com/acme/web-app/actions/runs/55"
        );
    }

    #[tokio::test]
    async fn list_pipelines_without_remote_ref_errors_without_request() {
        // No mock is mounted: if a request were made, wiremock would 404 and the body parse
        // would fail with a different error. A guard-before-request returns Network immediately.
        let server = MockServer::start().await;
        let err = provider(&server).list_pipelines(7, None).await.unwrap_err();
        assert!(
            matches!(err, ProviderError::Network(_)),
            "missing remote_ref must error without a request, got {err:?}"
        );
    }

    #[tokio::test]
    async fn list_jobs_unwraps_jobs_and_maps_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/repos/acme/web-app/actions/runs/55/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_count": 2,
                "jobs": [
                    {"id": 100, "name": "build", "status": "completed", "conclusion": "success",
                     "html_url": "https://github.com/acme/web-app/actions/runs/55/job/100"},
                    {"id": 101, "name": "test", "status": "in_progress", "conclusion": null,
                     "html_url": "https://github.com/acme/web-app/actions/runs/55/job/101"}
                ]
            })))
            .mount(&server)
            .await;

        let jobs = provider(&server)
            .list_jobs(7, Some("acme/web-app"), 55)
            .await
            .unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].name, "build");
        assert_eq!(jobs[0].status, PipelineStatus::Success);
        // GitHub has no stage concept.
        assert_eq!(jobs[0].stage, "");
        assert_eq!(
            jobs[0].web_url,
            "https://github.com/acme/web-app/actions/runs/55/job/100"
        );
        assert_eq!(jobs[1].name, "test");
        assert_eq!(jobs[1].status, PipelineStatus::Running);
    }

    #[tokio::test]
    async fn http_500_is_http_error() {
        let server = MockServer::start().await;
        // The repo metadata fetch is the first request; a 500 there surfaces as Http(500).
        Mock::given(method("GET"))
            .and(path("/api/v3/repos/acme/web-app"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let err = provider(&server)
            .list_pipelines(7, Some("acme/web-app"))
            .await
            .unwrap_err();
        assert_eq!(err, ProviderError::Http(500));
    }

    #[tokio::test]
    async fn token_health_reads_expiration_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(
                        "GitHub-Authentication-Token-Expiration",
                        "2026-08-15 14:23:01 UTC",
                    )
                    .set_body_json(serde_json::json!({"login": "octocat"})),
            )
            .mount(&server)
            .await;

        let health = provider(&server).token_health().await.unwrap();
        assert_eq!(
            health.expires_at.as_deref(),
            Some("2026-08-15 14:23:01 UTC")
        );
    }

    #[tokio::test]
    async fn token_health_no_header_is_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/user"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"login": "octocat"})),
            )
            .mount(&server)
            .await;

        let health = provider(&server).token_health().await.unwrap();
        assert_eq!(health.expires_at, None);
    }

    #[tokio::test]
    async fn token_health_401_is_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = provider(&server).token_health().await.unwrap_err();
        assert_eq!(err, ProviderError::Unauthorized);
    }

    #[tokio::test]
    async fn token_health_403_is_http_not_unauthorized() {
        // GitHub returns 403 for rate limiting; token_health must NOT treat it as a dead token
        // (this is the bug the reviewers flagged: do not reuse classify() here).
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v3/user"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let err = provider(&server).token_health().await.unwrap_err();
        assert_eq!(err, ProviderError::Http(403));
    }
}
