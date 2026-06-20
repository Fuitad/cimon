//! Provider abstraction: each CI provider implements [`Provider`], mapping its API onto
//! the normalized [`crate::model`] types. GitLab is the first implementation (Task 3);
//! GitHub will be a second, with no change required downstream of this trait.

use serde::Serialize;

use crate::model::{Identity, Job, Pipeline};

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

    /// List recent pipelines for a project, newest first.
    async fn list_pipelines(&self, project_id: u64) -> Result<Vec<Pipeline>, ProviderError>;

    /// List the jobs of a pipeline (for job-level notifications).
    async fn list_jobs(&self, project_id: u64, pipeline_id: u64)
        -> Result<Vec<Job>, ProviderError>;
}
