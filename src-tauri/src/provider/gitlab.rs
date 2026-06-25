//! GitLab implementation of the [`Provider`] trait.
//!
//! Talks to `<base_url>/api/v4` with a `PRIVATE-TOKEN` header. The base URL is validated at
//! the command layer (Task 5) and the shared client disables cross-host redirects, so a
//! token is only ever sent to the validated host.

use serde::Deserialize;

use crate::model::{Identity, Job, Pipeline, PipelineStatus};
use crate::provider::{DiscoveredProject, Provider, ProviderError, TokenHealth};
use crate::secrets::SecretToken;

const PER_PAGE: u32 = 100;
/// Hard ceiling on pages fetched, so a misbehaving server cannot drive an unbounded loop.
const MAX_PAGES: u32 = 1000;

/// A GitLab provider bound to one account's base URL and token.
pub struct GitlabProvider {
    client: reqwest::Client,
    /// Instance base URL, without a trailing slash and without `/api/v4`
    /// (e.g. `https://gitlab.com`).
    base_url: String,
    /// Held as a `SecretToken` so the copy zeroizes on drop and never appears in Debug.
    token: SecretToken,
}

impl GitlabProvider {
    pub fn new(
        client: reqwest::Client,
        base_url: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        GitlabProvider {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: SecretToken::new(token),
        }
    }

    fn url(&self, path_and_query: &str) -> String {
        format!("{}/api/v4{}", self.base_url, path_and_query)
    }

    async fn get(&self, path_and_query: &str) -> Result<reqwest::Response, ProviderError> {
        self.client
            .get(self.url(path_and_query))
            .header("PRIVATE-TOKEN", self.token.expose())
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
        401 | 403 => Some(ProviderError::Unauthorized),
        c => Some(ProviderError::Http(c)),
    }
}

fn redact_json_err(e: reqwest::Error) -> ProviderError {
    ProviderError::Network(e.without_url().to_string())
}

#[derive(Deserialize)]
struct GlUser {
    username: String,
    name: Option<String>,
    email: Option<String>,
}

fn default_true() -> bool {
    true
}

/// The `/personal_access_tokens/self` introspection response (only the fields token_health needs).
/// `active`/`revoked` default defensively so a future response that omits them is not read as dead.
#[derive(Deserialize)]
struct GlPatSelf {
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default = "default_true")]
    active: bool,
    #[serde(default)]
    revoked: bool,
}

#[derive(Deserialize)]
struct GlNamespace {
    full_path: String,
}

#[derive(Deserialize)]
struct GlProject {
    id: u64,
    name: String,
    web_url: String,
    /// Present in the `simple=true` project representation; absent only defensively.
    namespace: Option<GlNamespace>,
    /// The project's default branch. Present on the full single-project endpoint; absent on the
    /// lighter `simple=true` discovery listing, hence `Option`.
    default_branch: Option<String>,
}

#[derive(Deserialize)]
struct GlJob {
    id: u64,
    name: String,
    status: String,
    stage: String,
    web_url: String,
}

#[derive(Deserialize)]
struct GlPipeline {
    id: u64,
    status: String,
    #[serde(rename = "ref")]
    ref_: Option<String>,
    sha: Option<String>,
    web_url: String,
    updated_at: Option<String>,
}

impl Provider for GitlabProvider {
    async fn validate_token(&self) -> Result<Identity, ProviderError> {
        let resp = self.get("/user").await?;
        if let Some(err) = classify(resp.status().as_u16()) {
            return Err(err);
        }
        let u: GlUser = resp.json().await.map_err(redact_json_err)?;
        Ok(Identity {
            username: u.username,
            name: u.name,
            email: u.email,
        })
    }

    async fn token_health(&self) -> Result<TokenHealth, ProviderError> {
        // Liveness via /user, which works for ANY GitLab token type (personal AND project access
        // tokens). A 401 here is a genuinely dead token regardless of type; any other non-2xx is
        // transient (caller keeps prior state). This is the fix for project access tokens being
        // wrongly auth-failed: /personal_access_tokens/self (a personal-token-only endpoint) 401s
        // for a perfectly valid project token, so it must NOT be the liveness signal.
        let resp = self.get("/user").await?;
        match resp.status().as_u16() {
            200..=299 => {}
            401 => return Err(ProviderError::Unauthorized),
            c => return Err(ProviderError::Http(c)),
        }
        // Liveness confirmed. Best-effort expiry from the PAT self-introspection endpoint. A 2xx
        // that reports the token revoked/inactive is still a dead token (defense in depth). Any
        // other outcome (401 for a project token, 403 missing read_api scope, 404 pre-16.0 instance,
        // or a transient blip) degrades to "expiry unknown" rather than failing the whole check.
        let expires_at = match self.get("/personal_access_tokens/self").await {
            Ok(r) if (200..300).contains(&r.status().as_u16()) => {
                let pat: GlPatSelf = r.json().await.map_err(redact_json_err)?;
                if pat.revoked || !pat.active {
                    return Err(ProviderError::Unauthorized);
                }
                pat.expires_at
            }
            _ => None,
        };
        Ok(TokenHealth { expires_at })
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
                    "/projects?membership=true&simple=true&per_page={PER_PAGE}&page={page}"
                ))
                .await?;
            if let Some(err) = classify(resp.status().as_u16()) {
                return Err(err);
            }
            // Read the pagination signal before the body consumes the response.
            let has_next_header = resp.headers().contains_key("x-next-page");
            let next_page_val = resp
                .headers()
                .get("x-next-page")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_default();

            let projects: Vec<GlProject> = resp.json().await.map_err(redact_json_err)?;
            let count = projects.len();
            out.extend(projects.into_iter().map(|p| DiscoveredProject {
                id: p.id,
                name: p.name,
                web_url: p.web_url,
                group: p.namespace.map(|n| n.full_path).unwrap_or_default(),
                // GitLab addresses projects by numeric id, so no provider-specific ref.
                remote_ref: None,
            }));

            if has_next_header {
                // Trust the header when present.
                if next_page_val.is_empty() {
                    break;
                }
                match next_page_val.parse::<u32>() {
                    // Require strict forward progress so a constant/non-advancing
                    // x-next-page cannot drive an infinite loop.
                    Ok(n) if n > page => page = n,
                    _ => break,
                }
            } else {
                // Fallback for instances that omit x-next-page: stop on a short page.
                if count < PER_PAGE as usize {
                    break;
                }
                page += 1;
            }
        }
        Ok(out)
    }

    async fn list_pipelines(
        &self,
        project_id: u64,
        _remote_ref: Option<&str>, // GitLab addresses by project_id; the ref is unused.
    ) -> Result<Vec<Pipeline>, ProviderError> {
        // Scope the project's status to its default branch. GitLab lists pipelines across every
        // branch and merge request, so a feature-branch or MR pipeline would otherwise drive the
        // project's status. Fetch the project to learn its default branch, then filter to it.
        let proj_resp = self.get(&format!("/projects/{project_id}")).await?;
        if let Some(err) = classify(proj_resp.status().as_u16()) {
            return Err(err);
        }
        let meta: GlProject = proj_resp.json().await.map_err(redact_json_err)?;
        let default_branch = meta.default_branch.unwrap_or_default();

        let resp = self
            .get(&format!(
                "/projects/{project_id}/pipelines?ref={default_branch}&per_page=20&order_by=updated_at&sort=desc"
            ))
            .await?;
        if let Some(err) = classify(resp.status().as_u16()) {
            return Err(err);
        }
        let raw: Vec<GlPipeline> = resp.json().await.map_err(redact_json_err)?;
        Ok(raw
            .into_iter()
            .map(|p| Pipeline {
                id: p.id,
                project_id,
                status: PipelineStatus::from_gitlab(&p.status),
                ref_: p.ref_.unwrap_or_default(),
                sha: p.sha.unwrap_or_default(),
                web_url: p.web_url,
                updated_at: p.updated_at.unwrap_or_default(),
            })
            .collect())
    }

    async fn list_jobs(
        &self,
        project_id: u64,
        _remote_ref: Option<&str>, // GitLab addresses by project_id; the ref is unused.
        pipeline_id: u64,
    ) -> Result<Vec<Job>, ProviderError> {
        // per_page=100 covers any realistic pipeline in a single page; pipelines with more than
        // 100 jobs are not paginated here (a deliberate bound, see the plan's Task 12 SHORTCUT).
        let resp = self
            .get(&format!(
                "/projects/{project_id}/pipelines/{pipeline_id}/jobs?per_page=100"
            ))
            .await?;
        if let Some(err) = classify(resp.status().as_u16()) {
            return Err(err);
        }
        let raw: Vec<GlJob> = resp.json().await.map_err(redact_json_err)?;
        Ok(raw
            .into_iter()
            .map(|j| Job {
                id: j.id,
                name: j.name,
                status: PipelineStatus::from_gitlab(&j.status),
                stage: j.stage,
                web_url: j.web_url,
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

    fn provider(server: &MockServer) -> GitlabProvider {
        GitlabProvider::new(build_http_client(), server.uri(), "test-token")
    }

    /// Mount a 200 `/user` so `token_health` passes its liveness probe and proceeds to read expiry
    /// from `/personal_access_tokens/self`. Token health probes `/user` first (the token-type-
    /// agnostic liveness endpoint), so every expiry-path test needs this.
    async fn mount_user_ok(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "username": "alice"
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn validate_token_ok_returns_identity() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "username": "alice",
                "name": "Alice",
                "email": "alice@example.com"
            })))
            .mount(&server)
            .await;

        let id = provider(&server).validate_token().await.unwrap();
        assert_eq!(id.username, "alice");
        assert_eq!(id.email.as_deref(), Some("alice@example.com"));
    }

    #[tokio::test]
    async fn validate_token_401_is_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = provider(&server).validate_token().await.unwrap_err();
        assert_eq!(err, ProviderError::Unauthorized);
    }

    #[tokio::test]
    async fn list_projects_follows_x_next_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects"))
            .and(query_param("page", "1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-next-page", "2")
                    .set_body_json(serde_json::json!([
                        {"id": 1, "name": "alpha", "web_url": "http://x/alpha",
                         "namespace": {"full_path": "acme/web"}}
                    ])),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-next-page", "")
                    // No namespace: group falls back to empty rather than failing to parse.
                    .set_body_json(serde_json::json!([
                        {"id": 2, "name": "beta", "web_url": "http://x/beta"}
                    ])),
            )
            .mount(&server)
            .await;

        let projects = provider(&server).list_projects().await.unwrap();
        let ids: Vec<u64> = projects.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(projects[0].group, "acme/web");
        assert_eq!(
            projects[1].group, "",
            "missing namespace falls back to empty group"
        );
    }

    #[tokio::test]
    async fn list_pipelines_scopes_to_default_branch() {
        let server = MockServer::start().await;
        // The project is fetched first to learn its default branch.
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 7, "name": "app", "web_url": "http://x/p/7", "default_branch": "main"
            })))
            .mount(&server)
            .await;
        // Pipelines are then requested filtered to that branch.
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/pipelines"))
            .and(query_param("ref", "main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 10, "status": "failed", "ref": "main", "sha": "abc123",
                 "web_url": "http://x/p/10", "updated_at": "2026-06-20T00:00:00Z"}
            ])))
            .mount(&server)
            .await;

        let pipes = provider(&server).list_pipelines(7, None).await.unwrap();
        assert_eq!(pipes.len(), 1);
        assert_eq!(pipes[0].status, PipelineStatus::Failed);
        assert_eq!(pipes[0].project_id, 7);
        assert_eq!(pipes[0].ref_, "main");
        assert_eq!(pipes[0].web_url, "http://x/p/10");
    }

    #[tokio::test]
    async fn list_jobs_maps_status_name_stage_and_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/pipelines/42/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 100, "name": "build", "status": "success", "stage": "build",
                 "web_url": "https://gitlab.com/acme/app/-/jobs/100"},
                {"id": 101, "name": "test", "status": "running", "stage": "test",
                 "web_url": "https://gitlab.com/acme/app/-/jobs/101"}
            ])))
            .mount(&server)
            .await;

        let jobs = provider(&server).list_jobs(7, None, 42).await.unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].name, "build");
        assert_eq!(jobs[0].status, PipelineStatus::Success);
        assert_eq!(jobs[0].stage, "build");
        assert_eq!(jobs[0].web_url, "https://gitlab.com/acme/app/-/jobs/100");
        assert_eq!(jobs[1].name, "test");
        assert_eq!(jobs[1].status, PipelineStatus::Running);
    }

    #[tokio::test]
    async fn http_500_is_http_error() {
        let server = MockServer::start().await;
        // The project metadata fetch is the first request; a 500 there surfaces as Http(500).
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let err = provider(&server).list_pipelines(7, None).await.unwrap_err();
        assert_eq!(err, ProviderError::Http(500));
    }

    #[tokio::test]
    async fn token_health_self_returns_expiry() {
        let server = MockServer::start().await;
        mount_user_ok(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/v4/personal_access_tokens/self"))
            .and(header("PRIVATE-TOKEN", "test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 1, "active": true, "revoked": false, "expires_at": "2026-08-15"
            })))
            .mount(&server)
            .await;

        let health = provider(&server).token_health().await.unwrap();
        assert_eq!(health.expires_at.as_deref(), Some("2026-08-15"));
    }

    #[tokio::test]
    async fn token_health_project_token_alive_without_expiry() {
        // A valid GitLab *project* access token cannot self-introspect via
        // /personal_access_tokens/self (it is not a personal access token), so that endpoint 401s.
        // Liveness must come from /user (which works for any token type): the token is ALIVE with no
        // known expiry, NOT auth-failed. Regression guard for the "project token wrongly auth-failed"
        // bug.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "username": "project_bot"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/personal_access_tokens/self"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let health = provider(&server).token_health().await.unwrap();
        assert_eq!(
            health.expires_at, None,
            "a project token is alive (via /user) with no introspectable expiry"
        );
    }

    #[tokio::test]
    async fn token_health_null_expiry_is_none() {
        let server = MockServer::start().await;
        mount_user_ok(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/v4/personal_access_tokens/self"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 1, "active": true, "revoked": false, "expires_at": null
            })))
            .mount(&server)
            .await;

        let health = provider(&server).token_health().await.unwrap();
        assert_eq!(health.expires_at, None);
    }

    #[tokio::test]
    async fn token_health_revoked_is_unauthorized() {
        // Liveness passes (/user 200) but the self endpoint reports the token revoked/inactive: the
        // defense-in-depth secondary signal still flags it dead.
        let server = MockServer::start().await;
        mount_user_ok(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/v4/personal_access_tokens/self"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 1, "active": false, "revoked": true, "expires_at": "2020-01-01"
            })))
            .mount(&server)
            .await;

        let err = provider(&server).token_health().await.unwrap_err();
        assert_eq!(err, ProviderError::Unauthorized);
    }

    #[tokio::test]
    async fn token_health_user_401_is_unauthorized() {
        // A dead token (any type): /user is the liveness probe, so a 401 there is the auth-failure.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = provider(&server).token_health().await.unwrap_err();
        assert_eq!(err, ProviderError::Unauthorized);
    }

    #[tokio::test]
    async fn token_health_self_404_and_403_degrade_to_none() {
        // Liveness passes (/user 200); the self endpoint is unavailable (404 old instance) or
        // forbidden (403 missing scope): expiry unknown, but the token is NOT dead.
        for code in [404u16, 403] {
            let server = MockServer::start().await;
            mount_user_ok(&server).await;
            Mock::given(method("GET"))
                .and(path("/api/v4/personal_access_tokens/self"))
                .respond_with(ResponseTemplate::new(code))
                .mount(&server)
                .await;

            let health = provider(&server).token_health().await.unwrap();
            assert_eq!(
                health.expires_at, None,
                "code {code} should degrade to Ok(None)"
            );
        }
    }
}
