//! GitLab implementation of the [`Provider`] trait.
//!
//! Talks to `<base_url>/api/v4` with a `PRIVATE-TOKEN` header. The base URL is validated at
//! the command layer (Task 5) and the shared client disables cross-host redirects, so a
//! token is only ever sent to the validated host.

use serde::Deserialize;

use crate::model::{Identity, Job, Pipeline, PipelineStatus};
use crate::provider::{DiscoveredProject, Provider, ProviderError};
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
}

#[derive(Deserialize)]
struct GlJob {
    id: u64,
    name: String,
    status: String,
    stage: String,
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
        let resp = self
            .get(&format!(
                "/projects/{project_id}/pipelines?per_page=20&order_by=updated_at&sort=desc"
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
    async fn list_pipelines_maps_status_and_project_id() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/pipelines"))
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
    async fn list_jobs_maps_status_name_and_stage() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/pipelines/42/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": 100, "name": "build", "status": "success", "stage": "build"},
                {"id": 101, "name": "test", "status": "running", "stage": "test"}
            ])))
            .mount(&server)
            .await;

        let jobs = provider(&server).list_jobs(7, None, 42).await.unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].name, "build");
        assert_eq!(jobs[0].status, PipelineStatus::Success);
        assert_eq!(jobs[0].stage, "build");
        assert_eq!(jobs[1].name, "test");
        assert_eq!(jobs[1].status, PipelineStatus::Running);
    }

    #[tokio::test]
    async fn http_500_is_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/pipelines"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let err = provider(&server).list_pipelines(7, None).await.unwrap_err();
        assert_eq!(err, ProviderError::Http(500));
    }
}
