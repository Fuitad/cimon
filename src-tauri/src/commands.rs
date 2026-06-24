//! Tauri command bridge: the typed API the React frontend calls.
//!
//! Business logic lives in free functions (testable without a Tauri runtime); the
//! `#[tauri::command]` wrappers are thin adapters over [`AppState`]. Backend validation is
//! authoritative regardless of any frontend checks. Tokens are read from the keychain only at
//! the point of use and never returned to the frontend.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use url::Url;

use crate::config;
use crate::i18n;
use crate::model::{
    Account, Config, Identity, MonitoredProject, NotificationRules, PipelineStatus, ProviderKind,
    MAX_POLL_SECS, MIN_POLL_SECS,
};
use crate::poller::{ProjectKey, ProjectStatusView};
use crate::provider::{
    build_http_client, build_provider, DiscoveredProject, Provider, ProviderError,
};
use crate::secrets::{CachingTokenStore, KeyringStore, TokenStore};

/// Machine-readable error category so the frontend can localize the message by `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandErrorKind {
    Unauthorized,
    InvalidBaseUrl,
    InvalidInput,
    Network,
    Http,
    Storage,
    NotFound,
}

/// Error returned to the frontend. `kind` drives localized display; `message` is a fallback.
#[derive(Debug, Clone, Serialize)]
pub struct CommandError {
    pub kind: CommandErrorKind,
    pub message: String,
}

impl CommandError {
    fn new(kind: CommandErrorKind, message: impl Into<String>) -> Self {
        CommandError {
            kind,
            message: message.into(),
        }
    }
}

impl From<ProviderError> for CommandError {
    fn from(e: ProviderError) -> Self {
        match e {
            ProviderError::Unauthorized => {
                CommandError::new(CommandErrorKind::Unauthorized, "token unauthorized")
            }
            ProviderError::Http(c) => CommandError::new(
                CommandErrorKind::Http,
                format!("provider returned HTTP {c}"),
            ),
            ProviderError::Network(m) => CommandError::new(CommandErrorKind::Network, m),
        }
    }
}

fn storage_err(e: impl std::fmt::Display) -> CommandError {
    CommandError::new(CommandErrorKind::Storage, e.to_string())
}

/// Validate and normalize an instance base URL BEFORE any token is sent to it.
///
/// Requires `https` (unless `allow_insecure`), rejects embedded credentials, fragments, and
/// non-HTTP schemes, and strips path/query so the request target is exactly
/// `<scheme>://<host>[:port]`.
pub fn validate_base_url(input: &str, allow_insecure: bool) -> Result<String, CommandError> {
    let invalid = |msg: &str| CommandError::new(CommandErrorKind::InvalidBaseUrl, msg);
    let u = Url::parse(input.trim()).map_err(|_| invalid("not a valid URL"))?;

    match u.scheme() {
        "https" => {}
        "http" if allow_insecure => {}
        "http" => return Err(invalid("base URL must use https")),
        _ => return Err(invalid("base URL must use http or https")),
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err(invalid("base URL must not contain credentials"));
    }
    if u.fragment().is_some() {
        return Err(invalid("base URL must not contain a fragment"));
    }
    let host = u
        .host()
        .ok_or_else(|| invalid("base URL must have a host"))?;

    // Strip path/query/fragment: the request target is exactly scheme://host[:port].
    // `Host`'s Display wraps IPv6 literals in brackets (host_str() would drop them).
    let mut normalized = format!("{}://{}", u.scheme(), host);
    if let Some(port) = u.port() {
        normalized.push_str(&format!(":{port}"));
    }
    Ok(normalized)
}

// Dependencies are injected explicitly (http/tokens/cfg/path) so the logic is testable
// without a Tauri runtime; the arg count is a deliberate consequence of that.
#[allow(clippy::too_many_arguments)]
async fn add_account_logic(
    http: &reqwest::Client,
    tokens: &dyn TokenStore,
    cfg: &Mutex<Config>,
    cfg_path: &Path,
    provider: ProviderKind,
    label: String,
    base_url: String,
    token: String,
    allow_insecure: bool,
) -> Result<Identity, CommandError> {
    let normalized = validate_base_url(&base_url, allow_insecure)?;

    // Validate the token against the chosen provider BEFORE storing anything.
    let client = build_provider(provider, http.clone(), normalized.clone(), token.clone());
    let identity = client.validate_token().await?;

    let account_id = uuid::Uuid::new_v4().to_string();

    // Store the token first, then persist account metadata.
    tokens.store(&account_id, &token).map_err(storage_err)?;

    let account = Account {
        id: account_id.clone(),
        label,
        provider,
        base_url: normalized,
        identity: identity.clone(),
    };

    let save_result = {
        let mut guard = cfg.lock().unwrap();
        guard.accounts.push(account);
        config::save(cfg_path, &guard)
    };

    if let Err(e) = save_result {
        // Transactional rollback: undo the keychain write and the in-memory account so we
        // never leave an orphaned token or a half-written account.
        let _ = tokens.delete(&account_id);
        cfg.lock().unwrap().accounts.retain(|a| a.id != account_id);
        return Err(storage_err(e));
    }

    Ok(identity)
}

fn remove_account_logic(
    tokens: &dyn TokenStore,
    cfg: &Mutex<Config>,
    cfg_path: &Path,
    id: &str,
) -> Result<(), CommandError> {
    // Idempotent: tolerate the keychain entry already being gone.
    let _ = tokens.delete(id);
    let mut guard = cfg.lock().unwrap();
    guard.accounts.retain(|a| a.id != id);
    guard.monitored.retain(|m| m.account_id != id);
    config::save(cfg_path, &guard).map_err(storage_err)
}

/// A GitHub `remote_ref` must be a well-formed `owner/repo`: exactly two non-empty path segments,
/// no `.`/`..` traversal segment, no whitespace. This both guarantees a GitHub project can actually
/// be addressed (it has no usable numeric API address) and prevents a frontend- or config-supplied
/// value from splicing path-altering characters into the request URL.
fn is_valid_github_ref(s: &str) -> bool {
    let parts: Vec<&str> = s.split('/').collect();
    parts.len() == 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && *p != "." && *p != ".." && !p.contains(char::is_whitespace))
}

fn set_monitored_logic(
    cfg: &Mutex<Config>,
    cfg_path: &Path,
    account_id: &str,
    projects: Vec<MonitoredProject>,
) -> Result<(), CommandError> {
    let mut guard = cfg.lock().unwrap();
    // Backend validation (authoritative regardless of the frontend): a monitored GitHub project
    // must carry a well-formed owner/repo, otherwise it can never poll (and a raw value could
    // alter the request path). Validate the whole batch BEFORE mutating any state.
    let is_github = guard
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .map(|a| a.provider == ProviderKind::Github)
        .unwrap_or(false);
    if is_github {
        for p in &projects {
            if !p.remote_ref.as_deref().is_some_and(is_valid_github_ref) {
                return Err(CommandError::new(
                    CommandErrorKind::InvalidInput,
                    "GitHub project requires a valid owner/repo identifier",
                ));
            }
        }
    }
    // Replace only this account's selections; other accounts keep theirs.
    guard.monitored.retain(|m| m.account_id != account_id);
    for mut p in projects {
        // Enforce account scoping regardless of what the frontend sent.
        p.account_id = account_id.to_string();
        guard.monitored.push(p);
    }
    config::save(cfg_path, &guard).map_err(storage_err)
}

fn set_poll_interval_logic(
    cfg: &Mutex<Config>,
    cfg_path: &Path,
    secs: u64,
) -> Result<(), CommandError> {
    if !(MIN_POLL_SECS..=MAX_POLL_SECS).contains(&secs) {
        return Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            format!("interval must be between {MIN_POLL_SECS} and {MAX_POLL_SECS} seconds"),
        ));
    }
    let mut guard = cfg.lock().unwrap();
    guard.poll_interval_secs = secs;
    config::save(cfg_path, &guard).map_err(storage_err)
}

fn set_locale_logic(cfg: &Mutex<Config>, cfg_path: &Path, code: &str) -> Result<(), CommandError> {
    if !i18n::is_supported(code) {
        return Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            format!("unsupported locale: {code}"),
        ));
    }
    {
        let mut guard = cfg.lock().unwrap();
        guard.locale = Some(code.to_string());
        config::save(cfg_path, &guard).map_err(storage_err)?;
    }
    // Apply globally so the poller/tray (which read the global locale) localize after the
    // change. The tray rebuild on locale change is wired in Tasks 8/11.
    rust_i18n::set_locale(code);
    Ok(())
}

async fn list_discovered_logic(
    http: &reqwest::Client,
    tokens: &dyn TokenStore,
    cfg: &Mutex<Config>,
    account_id: &str,
) -> Result<Vec<DiscoveredProject>, CommandError> {
    // Read the account base URL and provider kind under the lock, then release it before any await.
    let (base_url, kind) = {
        let guard = cfg.lock().unwrap();
        guard
            .accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| (a.base_url.clone(), a.provider))
            .ok_or_else(|| CommandError::new(CommandErrorKind::NotFound, "account not found"))?
    };
    let token = tokens
        .get(account_id)
        .map_err(storage_err)?
        .ok_or_else(|| CommandError::new(CommandErrorKind::NotFound, "no token for account"))?;
    let provider = build_provider(kind, http.clone(), base_url, token.expose().to_string());
    Ok(provider.list_projects().await?)
}

/// A monitored project joined with its latest status, for the tray popover panel. The panel
/// fetches a `Vec<PanelProject>` (one per monitored project, in config order) via
/// [`get_project_statuses`] and refreshes on the `status-updated` event.
#[derive(Debug, Clone, Serialize)]
pub struct PanelProject {
    pub account_id: String,
    /// The account's user-given label (may be empty; the panel falls back to provider/host).
    pub account_label: String,
    pub provider: ProviderKind,
    /// The account's instance base URL, for a host fallback when the label is empty.
    pub base_url: String,
    pub project_id: u64,
    pub name: String,
    pub web_url: String,
    /// `None` until the first poll observes this project (or when it has no current pipeline): the
    /// panel renders that as a neutral "checking" row rather than a fabricated status.
    pub status: Option<PipelineStatus>,
    pub branch: String,
    /// The latest pipeline's `updated_at` (RFC3339), or `None` when never polled. Rendered relative.
    pub updated_at: Option<String>,
    /// `true` when the most recent poll FAILED: status/branch are last-known, shown as offline.
    pub stale: bool,
}

/// Shared application state managed by Tauri and used by all commands.
///
/// `config` is an `Arc<Mutex<..>>` so the background poller (Task 11) can share the exact same
/// config the commands mutate, with no risk of the two drifting.
pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub http: reqwest::Client,
    /// Shared (with the poller) so the keychain is read at most once per account per run.
    pub tokens: Arc<dyn TokenStore>,
    pub config_path: PathBuf,
    /// Latest per-project status snapshot, written by the poller each tick and read by the tray
    /// to render per-project rows. Empty until the first poll completes.
    pub project_status: Arc<Mutex<HashMap<ProjectKey, ProjectStatusView>>>,
}

impl AppState {
    /// Build state from the app config directory: load config, apply its locale, and set up the
    /// HTTP client and keychain store.
    pub fn bootstrap(config_dir: PathBuf) -> Self {
        let config_path = config_dir.join("config.json");
        let cfg = config::load(&config_path);
        i18n::apply(&cfg);
        let tokens: Arc<dyn TokenStore> =
            Arc::new(CachingTokenStore::new(Box::new(KeyringStore::new())));
        AppState {
            config: Arc::new(Mutex::new(cfg)),
            http: build_http_client(),
            tokens,
            config_path,
            project_status: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[tauri::command]
pub async fn add_account(
    state: tauri::State<'_, AppState>,
    provider: ProviderKind,
    label: String,
    base_url: String,
    token: String,
) -> Result<Identity, CommandError> {
    add_account_logic(
        &state.http,
        &*state.tokens,
        &state.config,
        &state.config_path,
        provider,
        label,
        base_url,
        token,
        false,
    )
    .await
}

#[tauri::command]
pub fn remove_account(state: tauri::State<'_, AppState>, id: String) -> Result<(), CommandError> {
    remove_account_logic(&*state.tokens, &state.config, &state.config_path, &id)
}

#[tauri::command]
pub fn list_accounts(state: tauri::State<'_, AppState>) -> Vec<Account> {
    state.config.lock().unwrap().accounts.clone()
}

#[tauri::command]
pub async fn list_discovered_projects(
    state: tauri::State<'_, AppState>,
    account_id: String,
) -> Result<Vec<DiscoveredProject>, CommandError> {
    list_discovered_logic(&state.http, &*state.tokens, &state.config, &account_id).await
}

#[tauri::command]
pub fn get_config(state: tauri::State<'_, AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_monitored_projects(state: tauri::State<'_, AppState>) -> Vec<MonitoredProject> {
    state.config.lock().unwrap().monitored.clone()
}

#[tauri::command]
pub fn set_monitored_projects(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    account_id: String,
    projects: Vec<MonitoredProject>,
) -> Result<(), CommandError> {
    set_monitored_logic(&state.config, &state.config_path, &account_id, projects)?;
    // The tray menu no longer lists projects (they live in the popover panel), so nudge an open
    // panel to re-fetch instead of rebuilding the menu.
    crate::panel::notify_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn set_notification_rules(
    state: tauri::State<'_, AppState>,
    rules: NotificationRules,
) -> Result<(), CommandError> {
    let mut guard = state.config.lock().unwrap();
    guard.rules = rules;
    config::save(&state.config_path, &guard).map_err(storage_err)
}

#[tauri::command]
pub fn set_poll_interval(state: tauri::State<'_, AppState>, secs: u64) -> Result<(), CommandError> {
    set_poll_interval_logic(&state.config, &state.config_path, secs)
}

#[tauri::command]
pub fn set_locale(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    code: String,
) -> Result<(), CommandError> {
    set_locale_logic(&state.config, &state.config_path, &code)?;
    crate::tray::refresh(&app); // retranslate the tray menu now (it reads the global locale)
    Ok(())
}

#[tauri::command]
pub fn set_launch_at_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), CommandError> {
    use tauri_plugin_autostart::ManagerExt;
    {
        let mut guard = state.config.lock().unwrap();
        guard.launch_at_login = enabled;
        config::save(&state.config_path, &guard).map_err(storage_err)?;
    }
    let autostart = app.autolaunch();
    let result = if enabled {
        autostart.enable()
    } else {
        autostart.disable()
    };
    result.map_err(|e| CommandError::new(CommandErrorKind::Storage, e.to_string()))
}

/// Monitored projects joined with their latest status, for the tray popover panel. Returns one
/// entry per monitored project in config order; a project not yet observed by the poller carries a
/// `None` status (a "checking" row). The frontend groups by account when more than one exists.
#[tauri::command]
pub fn get_project_statuses(state: tauri::State<'_, AppState>) -> Vec<PanelProject> {
    let cfg = state.config.lock().unwrap();
    let snapshot = state.project_status.lock().unwrap();
    cfg.monitored
        .iter()
        .map(|mp| {
            let acct = cfg.accounts.iter().find(|a| a.id == mp.account_id);
            let view = snapshot.get(&(mp.account_id.clone(), mp.project_id));
            PanelProject {
                account_id: mp.account_id.clone(),
                account_label: acct.map(|a| a.label.clone()).unwrap_or_default(),
                provider: acct.map(|a| a.provider).unwrap_or(ProviderKind::Gitlab),
                base_url: acct.map(|a| a.base_url.clone()).unwrap_or_default(),
                project_id: mp.project_id,
                name: mp.name.clone(),
                web_url: mp.web_url.clone(),
                status: view.map(|v| v.status),
                branch: view.map(|v| v.branch.clone()).unwrap_or_default(),
                updated_at: view.map(|v| v.updated_at.clone()),
                stale: view.is_some_and(|v| v.stale),
            }
        })
        .collect()
}

/// Open a monitored project's pipeline page in the default browser, then hide the panel. The URL
/// comes from the panel (a monitored project's `web_url`), but it is invoked from the webview, so
/// the scheme is validated here (http/https only) before handing it to the OS opener.
#[tauri::command]
pub fn open_project_url(app: tauri::AppHandle, url: String) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;
    let parsed = Url::parse(&url)
        .map_err(|_| CommandError::new(CommandErrorKind::InvalidInput, "not a valid URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(CommandError::new(
            CommandErrorKind::InvalidInput,
            "URL must be http or https",
        ));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| CommandError::new(CommandErrorKind::Network, e.to_string()))?;
    crate::panel::hide(&app);
    Ok(())
}

/// Open the settings window (and reveal the macOS dock icon) and hide the panel.
#[tauri::command]
pub fn show_settings_window(app: tauri::AppHandle) {
    crate::window::show_main(&app);
    crate::panel::hide(&app);
}

/// Quit the application (the panel's Quit action; mirrors the tray fallback menu's Quit).
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// Hide the panel (used by the panel's Escape key and after navigating away).
#[tauri::command]
pub fn hide_panel(app: tauri::AppHandle) {
    crate::panel::hide(&app);
}

/// Resize the panel to fit its measured content height (clamped in `panel`), then re-anchor it.
/// Called by the panel after it renders so the popover hugs its content yet caps and scrolls.
#[tauri::command]
pub fn set_panel_height(app: tauri::AppHandle, height: f64) {
    crate::panel::set_height(&app, height);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Config;
    use crate::secrets::MemoryTokenStore;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "cimon-cmd-{tag}-{}/config.json",
            std::process::id()
        ))
    }

    #[test]
    fn validate_base_url_accepts_https_and_strips_path() {
        assert_eq!(
            validate_base_url("https://gitlab.com/foo/bar?x=1", false).unwrap(),
            "https://gitlab.com"
        );
    }

    #[test]
    fn validate_base_url_rejects_http_by_default() {
        let e = validate_base_url("http://gitlab.com", false).unwrap_err();
        assert_eq!(e.kind, CommandErrorKind::InvalidBaseUrl);
    }

    #[test]
    fn validate_base_url_allows_http_when_opted_in() {
        assert_eq!(
            validate_base_url("http://localhost:8080", true).unwrap(),
            "http://localhost:8080"
        );
    }

    #[test]
    fn validate_base_url_rejects_credentials() {
        assert!(validate_base_url("https://user:pass@gitlab.com", false).is_err());
    }

    #[test]
    fn validate_base_url_rejects_garbage_and_other_schemes() {
        assert!(validate_base_url("not a url", false).is_err());
        assert!(validate_base_url("ftp://gitlab.com", false).is_err());
    }

    #[test]
    fn validate_base_url_preserves_port() {
        assert_eq!(
            validate_base_url("https://gl.example.com:8443/", false).unwrap(),
            "https://gl.example.com:8443"
        );
    }

    #[test]
    fn validate_base_url_brackets_ipv6_host() {
        assert_eq!(
            validate_base_url("https://[2001:db8::1]:8443/api", false).unwrap(),
            "https://[2001:db8::1]:8443"
        );
    }

    #[tokio::test]
    async fn add_account_valid_stores_token_and_persists_without_secret() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "username": "alice", "name": "Alice", "email": null
            })))
            .mount(&server)
            .await;

        let path = temp_path("add-ok");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let tokens = MemoryTokenStore::new();
        let cfg = Mutex::new(Config::default());

        // server.uri() is http://127.0.0.1:PORT, so allow_insecure is required here.
        let id = add_account_logic(
            &build_http_client(),
            &tokens,
            &cfg,
            &path,
            ProviderKind::Gitlab,
            "work".into(),
            server.uri(),
            "secret-tok".into(),
            true,
        )
        .await
        .unwrap();

        assert_eq!(id.username, "alice");
        assert_eq!(cfg.lock().unwrap().accounts.len(), 1);
        let acct_id = cfg.lock().unwrap().accounts[0].id.clone();
        assert_eq!(
            tokens.get(&acct_id).unwrap().unwrap().expose(),
            "secret-tok"
        );
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            !saved.contains("secret-tok"),
            "token leaked into config file"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn add_account_invalid_token_writes_nothing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let path = temp_path("add-bad");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let tokens = MemoryTokenStore::new();
        let cfg = Mutex::new(Config::default());

        let err = add_account_logic(
            &build_http_client(),
            &tokens,
            &cfg,
            &path,
            ProviderKind::Gitlab,
            "x".into(),
            server.uri(),
            "bad".into(),
            true,
        )
        .await
        .unwrap_err();

        assert_eq!(err.kind, CommandErrorKind::Unauthorized);
        assert_eq!(cfg.lock().unwrap().accounts.len(), 0);
        assert_eq!(tokens.count(), 0);
        assert!(!path.exists(), "no config should be written on failure");
    }

    #[tokio::test]
    async fn add_account_rolls_back_token_when_save_fails() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "username": "a", "name": null, "email": null
            })))
            .mount(&server)
            .await;

        // Make the config path unwritable: its parent is an existing FILE, so create_dir_all
        // (inside config::save) fails, forcing the rollback path.
        let base = std::env::temp_dir().join(format!("cimon-rb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::write(&base, "not a dir").unwrap();
        let bad_path = base.join("config.json");

        let tokens = MemoryTokenStore::new();
        let cfg = Mutex::new(Config::default());

        let err = add_account_logic(
            &build_http_client(),
            &tokens,
            &cfg,
            &bad_path,
            ProviderKind::Gitlab,
            "x".into(),
            server.uri(),
            "tok".into(),
            true,
        )
        .await
        .unwrap_err();

        assert_eq!(err.kind, CommandErrorKind::Storage);
        assert_eq!(cfg.lock().unwrap().accounts.len(), 0, "account rolled back");
        assert_eq!(tokens.count(), 0, "token rolled back, none orphaned");
        std::fs::remove_file(&base).ok();
    }

    #[tokio::test]
    async fn add_account_github_stores_github_account() {
        let server = MockServer::start().await;
        // GitHub validates via /user (GHE path here, since server.uri() is an IP host) with a
        // Bearer token. A GitLab provider would request /api/v4/user instead, so a stored Github
        // account with this identity proves the provider dispatch routed to GithubProvider.
        Mock::given(method("GET"))
            .and(path("/api/v3/user"))
            .and(header("Authorization", "Bearer gh-tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "login": "octocat", "name": "The Octocat", "email": null
            })))
            .mount(&server)
            .await;

        let path = temp_path("add-github");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let tokens = MemoryTokenStore::new();
        let cfg = Mutex::new(Config::default());

        let id = add_account_logic(
            &build_http_client(),
            &tokens,
            &cfg,
            &path,
            ProviderKind::Github,
            "gh".into(),
            server.uri(),
            "gh-tok".into(),
            true,
        )
        .await
        .unwrap();

        assert_eq!(id.username, "octocat");
        let guard = cfg.lock().unwrap();
        assert_eq!(guard.accounts.len(), 1);
        assert_eq!(guard.accounts[0].provider, ProviderKind::Github);
        drop(guard);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn set_poll_interval_rejects_zero_and_out_of_range() {
        let path = temp_path("interval");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let cfg = Mutex::new(Config::default());
        assert_eq!(
            set_poll_interval_logic(&cfg, &path, 0).unwrap_err().kind,
            CommandErrorKind::InvalidInput
        );
        assert!(set_poll_interval_logic(&cfg, &path, MAX_POLL_SECS + 1).is_err());
        set_poll_interval_logic(&cfg, &path, 60).unwrap();
        assert_eq!(cfg.lock().unwrap().poll_interval_secs, 60);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn set_monitored_is_account_scoped() {
        let path = temp_path("monitored");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let cfg = Mutex::new(Config::default());
        let mk = |acct: &str, id: u64| MonitoredProject {
            account_id: acct.into(),
            project_id: id,
            name: "p".into(),
            web_url: "u".into(),
            remote_ref: None,
        };
        set_monitored_logic(&cfg, &path, "acctA", vec![mk("acctA", 1)]).unwrap();
        set_monitored_logic(&cfg, &path, "acctB", vec![mk("acctB", 1)]).unwrap();
        // Same project id 1 under two different accounts stays distinct.
        assert_eq!(cfg.lock().unwrap().monitored.len(), 2);
        // Re-setting acctA replaces only acctA's entries.
        set_monitored_logic(&cfg, &path, "acctA", vec![mk("acctA", 2), mk("acctA", 3)]).unwrap();
        let guard = cfg.lock().unwrap();
        assert_eq!(
            guard
                .monitored
                .iter()
                .filter(|m| m.account_id == "acctA")
                .count(),
            2
        );
        assert_eq!(
            guard
                .monitored
                .iter()
                .filter(|m| m.account_id == "acctB")
                .count(),
            1
        );
        drop(guard);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn set_monitored_rejects_github_project_without_valid_remote_ref() {
        let path = temp_path("gh-monitored");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let cfg = Mutex::new(Config {
            accounts: vec![Account {
                id: "gh".into(),
                label: "l".into(),
                provider: ProviderKind::Github,
                base_url: "https://github.com".into(),
                identity: Identity {
                    username: "u".into(),
                    name: None,
                    email: None,
                },
            }],
            ..Config::default()
        });
        let mk = |remote_ref: Option<&str>| MonitoredProject {
            account_id: "gh".into(),
            project_id: 1,
            name: "p".into(),
            web_url: "u".into(),
            remote_ref: remote_ref.map(str::to_string),
        };
        // Missing remote_ref: a GitHub project can never be addressed (owner/repo) -> rejected.
        assert_eq!(
            set_monitored_logic(&cfg, &path, "gh", vec![mk(None)])
                .unwrap_err()
                .kind,
            CommandErrorKind::InvalidInput
        );
        // Malformed / path-altering remote_ref -> rejected (no raw splice into the request path).
        assert!(set_monitored_logic(&cfg, &path, "gh", vec![mk(Some("../orgs/victim"))]).is_err());
        assert!(set_monitored_logic(&cfg, &path, "gh", vec![mk(Some("noslash"))]).is_err());
        assert!(set_monitored_logic(&cfg, &path, "gh", vec![mk(Some("owner/"))]).is_err());
        assert_eq!(
            cfg.lock().unwrap().monitored.len(),
            0,
            "nothing persisted on rejection"
        );
        // A well-formed owner/repo is accepted and persisted.
        set_monitored_logic(&cfg, &path, "gh", vec![mk(Some("acme/web-app"))]).unwrap();
        assert_eq!(cfg.lock().unwrap().monitored.len(), 1);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn set_locale_validates_and_persists() {
        let path = temp_path("locale");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let cfg = Mutex::new(Config::default());
        assert_eq!(
            set_locale_logic(&cfg, &path, "zz").unwrap_err().kind,
            CommandErrorKind::InvalidInput
        );
        set_locale_logic(&cfg, &path, "fr").unwrap();
        assert_eq!(cfg.lock().unwrap().locale.as_deref(), Some("fr"));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }
}
