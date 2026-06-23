//! Provider abstraction: each CI provider implements [`Provider`], mapping its API onto
//! the normalized [`crate::model`] types. GitLab is the first implementation, GitHub the
//! second. The trait carries an optional `remote_ref` (a provider-specific project address)
//! alongside the numeric `project_id`, so a provider addressed by something other than a
//! numeric id (GitHub's `owner/repo`) fits without changing the shared key.

use serde::Serialize;

use crate::model::{Identity, Job, Pipeline};

pub mod github;
pub mod gitlab;

/// Build the shared HTTP client used for all provider requests.
///
/// Redirects are disabled outright: a token-bearing request must never be transparently
/// re-sent to a different host (defense in depth on top of base-URL validation).
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("CIMon/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("failed to build HTTP client")
}

/// Errors a provider can return. Carries NO token material; messages are status-derived so
/// a token can never leak into a log or error surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Token rejected (HTTP 401/403).
    Unauthorized,
    /// Non-success HTTP response; the status code is carried for context.
    Http(u16),
    /// Transport/network failure (already-redacted message).
    Network(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Unauthorized => {
                write!(f, "authentication failed (token unauthorized)")
            }
            ProviderError::Http(code) => write!(f, "provider returned HTTP {code}"),
            ProviderError::Network(msg) => write!(f, "network error: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// A project discovered via the provider API, surfaced to the selection UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscoveredProject {
    pub id: u64,
    pub name: String,
    pub web_url: String,
    /// Owning group / namespace path (e.g. `acme/backend`), used to group the selection list.
    /// Empty when the provider reports no namespace.
    pub group: String,
    /// Provider-specific project ADDRESS carried into the monitored set (GitHub `owner/repo`).
    /// `None` for GitLab, which addresses projects by `id`. Not a git ref.
    pub remote_ref: Option<String>,
}

/// The read-only operations CIMon needs from a CI provider.
///
/// `dyn`-dispatch is intentionally not required: callers use concrete provider types (or a
/// dispatching enum), so native `async fn` in the trait is fine.
#[allow(async_fn_in_trait)]
pub trait Provider {
    /// Validate the configured token and resolve the authenticated identity.
    async fn validate_token(&self) -> Result<Identity, ProviderError>;

    /// List the projects the token can access (for the monitor-selection UI).
    async fn list_projects(&self) -> Result<Vec<DiscoveredProject>, ProviderError>;

    /// List recent pipelines for a project, newest first. `remote_ref` is the provider-specific
    /// project address (GitHub `owner/repo`); `None` for providers that address by `project_id`.
    async fn list_pipelines(
        &self,
        project_id: u64,
        remote_ref: Option<&str>,
    ) -> Result<Vec<Pipeline>, ProviderError>;

    /// List the jobs of a pipeline (for job-level notifications). `remote_ref` as above.
    async fn list_jobs(
        &self,
        project_id: u64,
        remote_ref: Option<&str>,
        pipeline_id: u64,
    ) -> Result<Vec<Job>, ProviderError>;
}

/// Dispatches the [`Provider`] trait to the concrete provider for an account's kind, so the
/// command and poller layers stay provider-agnostic (they hold an `AnyProvider`, not a concrete
/// type). This is the "dispatching enum" the trait doc refers to.
pub enum AnyProvider {
    Gitlab(gitlab::GitlabProvider),
    Github(github::GithubProvider),
}

/// Build the provider for an account's kind, bound to its base URL and token.
pub fn build_provider(
    kind: crate::model::ProviderKind,
    client: reqwest::Client,
    base_url: impl Into<String>,
    token: impl Into<String>,
) -> AnyProvider {
    use crate::model::ProviderKind;
    match kind {
        ProviderKind::Gitlab => {
            AnyProvider::Gitlab(gitlab::GitlabProvider::new(client, base_url, token))
        }
        ProviderKind::Github => {
            AnyProvider::Github(github::GithubProvider::new(client, base_url, token))
        }
    }
}

impl Provider for AnyProvider {
    async fn validate_token(&self) -> Result<Identity, ProviderError> {
        match self {
            AnyProvider::Gitlab(p) => p.validate_token().await,
            AnyProvider::Github(p) => p.validate_token().await,
        }
    }

    async fn list_projects(&self) -> Result<Vec<DiscoveredProject>, ProviderError> {
        match self {
            AnyProvider::Gitlab(p) => p.list_projects().await,
            AnyProvider::Github(p) => p.list_projects().await,
        }
    }

    async fn list_pipelines(
        &self,
        project_id: u64,
        remote_ref: Option<&str>,
    ) -> Result<Vec<Pipeline>, ProviderError> {
        match self {
            AnyProvider::Gitlab(p) => p.list_pipelines(project_id, remote_ref).await,
            AnyProvider::Github(p) => p.list_pipelines(project_id, remote_ref).await,
        }
    }

    async fn list_jobs(
        &self,
        project_id: u64,
        remote_ref: Option<&str>,
        pipeline_id: u64,
    ) -> Result<Vec<Job>, ProviderError> {
        match self {
            AnyProvider::Gitlab(p) => p.list_jobs(project_id, remote_ref, pipeline_id).await,
            AnyProvider::Github(p) => p.list_jobs(project_id, remote_ref, pipeline_id).await,
        }
    }
}
