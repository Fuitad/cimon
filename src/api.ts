import { invoke } from "@tauri-apps/api/core";

import type {
  Account,
  AccountTokenHealth,
  AppInfo,
  Config,
  DiscoveredProject,
  Identity,
  MonitoredProject,
  NotificationRules,
  PanelProject,
  ProviderKind,
  UiMode,
  UpdateState,
} from "./types";
import { DEFAULT_NOTIFICATION_RULES } from "./types";

// Tauri v2 maps camelCase JS keys to snake_case Rust parameters automatically.

// --- Design preview fixtures (dev only) ---------------------------------------------------
// When the app is opened in a plain browser during `npm run dev` (no Tauri shell, so
// `__TAURI_INTERNALS__` is absent), the commands below would all reject. To make the UI
// reviewable in a browser, dev builds return static fixtures instead. This whole block is
// behind `import.meta.env.DEV`, so it is dead-code-eliminated from production bundles and
// never runs inside the real app (where the Tauri globals are present). Append `?preview=empty`
// to the URL to preview the first-run (no accounts) states.
const PREVIEW =
  import.meta.env.DEV &&
  typeof window !== "undefined" &&
  !("__TAURI_INTERNALS__" in window) &&
  !("__TAURI__" in window);

const previewParam = (): string | null =>
  typeof window !== "undefined" ? new URLSearchParams(window.location.search).get("preview") : null;
const previewEmpty = () => previewParam() === "empty";
// `?preview=stress` exercises the hardening edge cases: very long names, deep namespace paths, a
// group with many projects, and a second account whose discovery fails (per-account error + retry).
const previewStress = () => previewParam() === "stress";
// `?preview=github` exercises the GitHub provider path: a GitHub account whose discovered repos
// carry `remote_ref` (owner/repo), so the provider selector and remote_ref toggle flow are reviewable.
const previewGithub = () => previewParam() === "github";

const FIXTURE_ACCOUNT: Account = {
  id: "acc-1",
  label: "Work GitLab",
  provider: "gitlab",
  base_url: "https://gitlab.com",
  identity: { username: "devuser", name: "Dev User", email: null },
};
const FIXTURE_PROJECTS: DiscoveredProject[] = [
  {
    id: 41,
    name: "web-app",
    web_url: "https://gitlab.com/acme/frontend/web-app",
    group: "acme/frontend",
  },
  {
    id: 45,
    name: "design-system",
    web_url: "https://gitlab.com/acme/frontend/design-system",
    group: "acme/frontend",
  },
  {
    id: 42,
    name: "api-gateway",
    web_url: "https://gitlab.com/acme/backend/api-gateway",
    group: "acme/backend",
  },
  {
    id: 46,
    name: "auth-service",
    web_url: "https://gitlab.com/acme/backend/auth-service",
    group: "acme/backend",
  },
  {
    id: 47,
    name: "billing",
    web_url: "https://gitlab.com/acme/backend/billing",
    group: "acme/backend",
  },
  {
    id: 43,
    name: "mobile-client",
    web_url: "https://gitlab.com/acme/mobile/mobile-client",
    group: "acme/mobile",
  },
  {
    id: 44,
    name: "terraform",
    web_url: "https://gitlab.com/acme/ops/terraform",
    group: "acme/ops",
  },
  { id: 48, name: "dotfiles", web_url: "https://gitlab.com/devuser/dotfiles", group: "" },
];
const FIXTURE_MONITORED: MonitoredProject[] = [
  {
    account_id: "acc-1",
    project_id: 41,
    name: "web-app",
    web_url: "https://gitlab.com/acme/frontend/web-app",
  },
  {
    account_id: "acc-1",
    project_id: 42,
    name: "api-gateway",
    web_url: "https://gitlab.com/acme/backend/api-gateway",
  },
];

// Stress fixtures (dev only, `?preview=stress`).
const STRESS_GROUP = "acme/platform/services/identity-and-access-management";
const STRESS_ACCOUNTS: Account[] = [
  FIXTURE_ACCOUNT,
  {
    id: "acc-2",
    label: "",
    provider: "gitlab",
    base_url: "https://gitlab.internal.engineering.very-long-corp-subdomain.example.com",
    identity: { username: "ci-bot-service-account", name: null, email: null },
  },
];
const STRESS_PROJECTS: DiscoveredProject[] = [
  {
    id: 900,
    name: "a-deliberately-very-long-monorepo-project-name-that-keeps-going-well-past-any-reasonable-row-width-to-test-truncation",
    web_url: "https://gitlab.com/acme/frontend/long-name",
    group: "acme/frontend",
  },
  {
    id: 901,
    name: "web-app",
    web_url: "https://gitlab.com/acme/frontend/web-app",
    group: "acme/frontend",
  },
  {
    id: 902,
    name: "mobile-client",
    web_url: "https://gitlab.com/acme/mobile/mobile-client",
    group: "acme/mobile",
  },
  { id: 903, name: "dotfiles", web_url: "https://gitlab.com/devuser/dotfiles", group: "" },
  ...Array.from({ length: 30 }, (_, i) => {
    const n = String(i + 1).padStart(2, "0");
    return {
      id: 1000 + i,
      name: `service-${n}`,
      web_url: `https://gitlab.com/${STRESS_GROUP}/service-${n}`,
      group: STRESS_GROUP,
    };
  }),
];
const STRESS_MONITORED: MonitoredProject[] = [
  {
    account_id: "acc-1",
    project_id: 900,
    name: "long-name",
    web_url: "https://gitlab.com/acme/frontend/long-name",
  },
  ...[0, 1, 2, 3, 4].map((i) => {
    const n = String(i + 1).padStart(2, "0");
    return {
      account_id: "acc-1",
      project_id: 1000 + i,
      name: `service-${n}`,
      web_url: `https://gitlab.com/${STRESS_GROUP}/service-${n}`,
    };
  }),
];

// GitHub preview fixtures (dev only, `?preview=github`). Repos carry `remote_ref` so the monitored
// selection persists the owner/repo slug, mirroring the real GitHub discovery shape.
const GITHUB_FIXTURE_ACCOUNT: Account = {
  id: "gh-1",
  label: "GitHub",
  provider: "github",
  base_url: "https://github.com",
  identity: { username: "octocat", name: "The Octocat", email: null },
};
const GITHUB_FIXTURE_PROJECTS: DiscoveredProject[] = [
  {
    id: 2001,
    name: "web-app",
    web_url: "https://github.com/acme/web-app",
    group: "acme",
    remote_ref: "acme/web-app",
  },
  {
    id: 2002,
    name: "api",
    web_url: "https://github.com/acme/api",
    group: "acme",
    remote_ref: "acme/api",
  },
  {
    id: 2003,
    name: "dotfiles",
    web_url: "https://github.com/octocat/dotfiles",
    group: "octocat",
    remote_ref: "octocat/dotfiles",
  },
];

// Panel preview fixtures (dev only). `?preview=empty` -> no monitored projects (the panel's
// empty/CTA state); `?preview=multi` -> two accounts so the per-account grouping is reviewable;
// default -> one account with a spread of statuses (success, running, failed, pending, stale,
// decayed offline, never-polled, no CI) so every row treatment is visible in a plain browser.
const ago = (mins: number): string => new Date(Date.now() - mins * 60_000).toISOString();
const PANEL_FIXTURE: PanelProject[] = [
  {
    account_id: "acc-1",
    account_label: "Work GitLab",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    project_id: 41,
    name: "web-app",
    web_url: "https://gitlab.com/acme/frontend/web-app",
    status: "failed",
    branch: "main",
    updated_at: ago(4),
    stale: false,
    no_pipelines: false,
    offline: false,
    auth_failed: false,
  },
  {
    account_id: "acc-1",
    account_label: "Work GitLab",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    project_id: 42,
    name: "api-gateway",
    web_url: "https://gitlab.com/acme/backend/api-gateway",
    status: "running",
    branch: "feature/checkout-v2",
    updated_at: ago(0),
    stale: false,
    no_pipelines: false,
    offline: false,
    auth_failed: false,
  },
  {
    account_id: "acc-1",
    account_label: "Work GitLab",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    project_id: 43,
    name: "auth-service",
    web_url: "https://gitlab.com/acme/backend/auth-service",
    status: "success",
    branch: "main",
    updated_at: ago(12),
    stale: false,
    no_pipelines: false,
    offline: false,
    auth_failed: false,
  },
  {
    account_id: "acc-1",
    account_label: "Work GitLab",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    project_id: 44,
    name: "design-system",
    web_url: "https://gitlab.com/acme/frontend/design-system",
    status: "pending",
    branch: "release/2.1",
    updated_at: ago(1),
    stale: false,
    no_pipelines: false,
    offline: false,
    auth_failed: false,
  },
  {
    account_id: "acc-1",
    account_label: "Work GitLab",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    project_id: 45,
    name: "terraform-infra",
    web_url: "https://gitlab.com/acme/ops/terraform-infra",
    status: "success",
    branch: "main",
    updated_at: ago(180),
    stale: true,
    no_pipelines: false,
    offline: false,
    auth_failed: false,
  },
  {
    account_id: "acc-1",
    account_label: "Work GitLab",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    project_id: 48,
    name: "batch-runner",
    web_url: "https://gitlab.com/acme/ops/batch-runner",
    // Decayed offline: last-known Running, but the server has been unreachable past the decay
    // window -- the row reads "Offline" and must not count as running in the headline.
    status: "running",
    branch: "main",
    updated_at: ago(1_200),
    stale: true,
    no_pipelines: false,
    offline: true,
    auth_failed: false,
  },
  {
    account_id: "acc-1",
    account_label: "Work GitLab",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    project_id: 46,
    name: "mobile-client",
    web_url: "https://gitlab.com/acme/mobile/mobile-client",
    status: null,
    branch: "",
    updated_at: null,
    stale: false,
    no_pipelines: false,
    offline: false,
    auth_failed: false,
  },
  {
    account_id: "acc-1",
    account_label: "Work GitLab",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    project_id: 47,
    name: "docs-site",
    web_url: "https://gitlab.com/acme/docs/docs-site",
    // Polled successfully but has no CI pipeline at all (no CI configured, or CI never ran): a
    // settled "no CI" row, distinct from mobile-client above which is still awaiting its first poll.
    status: null,
    branch: "",
    updated_at: null,
    stale: false,
    no_pipelines: true,
    offline: false,
    auth_failed: false,
  },
];
const PANEL_MULTI_FIXTURE: PanelProject[] = [
  ...PANEL_FIXTURE.slice(0, 3),
  {
    account_id: "gh-1",
    account_label: "",
    provider: "github",
    base_url: "https://github.com",
    project_id: 2001,
    name: "octobox",
    web_url: "https://github.com/octocat/octobox",
    status: "success",
    branch: "main",
    updated_at: ago(7),
    stale: false,
    no_pipelines: false,
    offline: false,
    auth_failed: false,
  },
  {
    account_id: "gh-1",
    account_label: "",
    provider: "github",
    base_url: "https://github.com",
    project_id: 2002,
    name: "a-deliberately-long-repository-name-to-exercise-row-truncation",
    web_url: "https://github.com/octocat/long",
    status: "failed",
    branch: "fix/very-long-branch-name-for-truncation-testing",
    updated_at: ago(2),
    stale: false,
    no_pipelines: false,
    offline: false,
    auth_failed: false,
  },
];

// `?preview=offline` reproduces a VPN-down scenario: projects that have never polled successfully
// (status null + stale) read as "can't connect", distinct from a freshly-added project still being
// polled for the first time (status null, not stale) which reads "checking".
const PANEL_OFFLINE_FIXTURE: PanelProject[] = [
  { ...PANEL_FIXTURE[2], status: null, branch: "", updated_at: null, stale: true },
  { ...PANEL_FIXTURE[1], status: null, branch: "", updated_at: null, stale: true },
  PANEL_FIXTURE[5], // never polled yet (status null, not stale) -> "checking"
  PANEL_FIXTURE[6], // polled, no CI at all (status null, not stale, no_pipelines) -> "no CI"
];

// `?preview=tokenhealth` exercises the token-health states: one account with a dead token
// (auth_failed, rendered distinct from offline) and one whose token expires in ~2 days (the warning
// indicator + the inline re-entry flow). Drives the TS-001/002/003 E2E scenarios.
const inDays = (days: number): string =>
  new Date(Date.now() + days * 86_400_000).toISOString().slice(0, 10);
const TOKENHEALTH_ACCOUNTS: Account[] = [
  {
    id: "th-dead",
    label: "Prod GitLab",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    identity: { username: "ci-bot", name: null, email: null },
  },
  {
    id: "th-exp",
    label: "GitHub",
    provider: "github",
    base_url: "https://github.com",
    identity: { username: "octocat", name: "The Octocat", email: null },
  },
];
const TOKENHEALTH_HEALTH: AccountTokenHealth[] = [
  { account_id: "th-dead", auth_failed: true, expires_at: null, expires_in_days: null },
  // The backend computes the day count; the preview mirrors it (~2 days out).
  { account_id: "th-exp", auth_failed: false, expires_at: inDays(2), expires_in_days: 2 },
];
const TOKENHEALTH_PANEL: PanelProject[] = [
  { ...PANEL_FIXTURE[0], account_id: "th-dead", account_label: "Prod GitLab", auth_failed: true },
  // A genuinely offline (stale) row on the same account, for visual contrast with auth_failed.
  { ...PANEL_FIXTURE[4], account_id: "th-dead", account_label: "Prod GitLab" },
  { ...PANEL_FIXTURE[2], account_id: "th-exp", account_label: "GitHub", provider: "github" },
];

const UPDATE_AVAILABLE: UpdateState = {
  status: "available",
  available: {
    version: "0.1.4",
    body: "Reliability fixes and updater support.",
    date: new Date().toISOString(),
    release_url: "https://github.com/Fuitad/cimon/releases/latest",
    self_updatable: true,
  },
  last_checked_at: String(Math.floor(Date.now() / 1000)),
  error: null,
  progress: null,
  dismissed_version: null,
};
const UPDATE_CURRENT: UpdateState = {
  status: "up_to_date",
  available: null,
  last_checked_at: String(Math.floor(Date.now() / 1000)),
  error: null,
  progress: null,
  dismissed_version: null,
};
const UPDATE_ERROR: UpdateState = {
  status: "error",
  available: null,
  last_checked_at: String(Math.floor(Date.now() / 1000)),
  error: "offline",
  progress: null,
  dismissed_version: null,
};

const previewUpdateState = (): UpdateState => {
  switch (previewParam()) {
    case "update":
      return UPDATE_AVAILABLE;
    case "update-linux":
      return {
        ...UPDATE_AVAILABLE,
        available: UPDATE_AVAILABLE.available
          ? { ...UPDATE_AVAILABLE.available, self_updatable: false }
          : null,
      };
    case "update-current":
      return UPDATE_CURRENT;
    case "update-error":
      return UPDATE_ERROR;
    default:
      return {
        status: "idle",
        available: null,
        last_checked_at: null,
        error: null,
        progress: null,
        dismissed_version: null,
      };
  }
};

export const getProjectStatuses = (): Promise<PanelProject[]> => {
  if (!PREVIEW) return invoke("get_project_statuses");
  if (previewEmpty()) return Promise.resolve([]);
  if (previewParam() === "multi") return Promise.resolve(PANEL_MULTI_FIXTURE);
  if (previewParam() === "offline") return Promise.resolve(PANEL_OFFLINE_FIXTURE);
  if (previewParam() === "tokenhealth") return Promise.resolve(TOKENHEALTH_PANEL);
  return Promise.resolve(PANEL_FIXTURE);
};

export const openProjectUrl = (accountId: string, projectId: number): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("open_project_url", { accountId, projectId });

export const appInfo = (): Promise<AppInfo> =>
  PREVIEW ? Promise.resolve({ version: "dev", commit: "abcdef1" }) : invoke("app_info");

export const showSettingsWindow = (): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("show_settings_window");

export const quitApp = (): Promise<void> => (PREVIEW ? Promise.resolve() : invoke("quit_app"));

export const hidePanel = (): Promise<void> => (PREVIEW ? Promise.resolve() : invoke("hide_panel"));

export const setPanelHeight = (height: number): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("set_panel_height", { height });

export const getUpdateState = (): Promise<UpdateState> =>
  PREVIEW ? Promise.resolve(previewUpdateState()) : invoke("get_update_state");

export const checkForUpdates = (): Promise<UpdateState> =>
  PREVIEW ? Promise.resolve(previewUpdateState()) : invoke("check_for_updates");

export const installUpdate = (): Promise<UpdateState> =>
  PREVIEW
    ? Promise.resolve({
        ...previewUpdateState(),
        status: "installing",
        progress: { downloaded: 64, total: 100 },
      })
    : invoke("install_update");

export const dismissUpdate = (): Promise<UpdateState> =>
  PREVIEW
    ? Promise.resolve({
        ...previewUpdateState(),
        dismissed_version: previewUpdateState().available?.version ?? null,
      })
    : invoke("dismiss_update");

export const openUpdateRelease = (fromPanel: boolean): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("open_update_release", { fromPanel });

export const addAccount = (
  provider: ProviderKind,
  label: string,
  baseUrl: string,
  token: string,
): Promise<Identity> => {
  if (PREVIEW) return Promise.resolve(FIXTURE_ACCOUNT.identity);
  // Typed args object so a dropped key (e.g. `provider`) is a compile error. The loosely-typed
  // invoke() second argument and the fixture-mode preview would otherwise hide the omission.
  const args: { provider: ProviderKind; label: string; baseUrl: string; token: string } = {
    provider,
    label,
    baseUrl,
    token,
  };
  return invoke("add_account", args);
};

export const removeAccount = (id: string): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("remove_account", { id });

export const listAccounts = (): Promise<Account[]> => {
  if (!PREVIEW) return invoke("list_accounts");
  if (previewEmpty()) return Promise.resolve([]);
  if (previewStress()) return Promise.resolve(STRESS_ACCOUNTS);
  if (previewGithub()) return Promise.resolve([GITHUB_FIXTURE_ACCOUNT]);
  if (previewParam() === "tokenhealth") return Promise.resolve(TOKENHEALTH_ACCOUNTS);
  return Promise.resolve([FIXTURE_ACCOUNT]);
};

// Preview-only: accounts whose token was "updated" this session, so getTokenHealth reflects the
// recovery in the UI (the real backend repopulates health from the next poll).
const previewRecovered = new Set<string>();

/** Per-account token health for the settings UI (auth-failed flag + expiry). */
export const getTokenHealth = (): Promise<AccountTokenHealth[]> => {
  if (!PREVIEW) return invoke("get_token_health");
  if (previewParam() === "tokenhealth")
    return Promise.resolve(
      TOKENHEALTH_HEALTH.map((h) =>
        previewRecovered.has(h.account_id) ? { ...h, auth_failed: false, expires_at: null } : h,
      ),
    );
  return Promise.resolve([]);
};

/** Replace the token for an existing account in place (re-validated server-side). */
export const updateAccountToken = (accountId: string, token: string): Promise<Identity> => {
  if (PREVIEW) {
    previewRecovered.add(accountId);
    return Promise.resolve(TOKENHEALTH_ACCOUNTS[0].identity);
  }
  const args: { accountId: string; token: string } = { accountId, token };
  return invoke("update_account_token", args);
};

export const listDiscoveredProjects = (accountId: string): Promise<DiscoveredProject[]> => {
  if (!PREVIEW) return invoke("list_discovered_projects", { accountId });
  if (previewStress()) {
    // The second account fails discovery, so the per-account error + Retry path is reviewable.
    if (accountId === "acc-2") {
      return Promise.reject(
        Object.assign(new Error("connection refused"), { kind: "network" as const }),
      );
    }
    return Promise.resolve(STRESS_PROJECTS);
  }
  if (previewGithub()) return Promise.resolve(GITHUB_FIXTURE_PROJECTS);
  return Promise.resolve(FIXTURE_PROJECTS);
};

export const getConfig = (): Promise<Config> =>
  PREVIEW
    ? Promise.resolve({
        accounts: previewEmpty()
          ? []
          : previewStress()
            ? STRESS_ACCOUNTS
            : previewGithub()
              ? [GITHUB_FIXTURE_ACCOUNT]
              : [FIXTURE_ACCOUNT],
        // GitHub preview starts with nothing monitored so the remote_ref toggle flow is reviewable.
        monitored: previewEmpty()
          ? []
          : previewStress()
            ? STRESS_MONITORED
            : previewGithub()
              ? []
              : FIXTURE_MONITORED,
        rules: DEFAULT_NOTIFICATION_RULES,
        poll_interval_secs: 30,
        launch_at_login: false,
        locale: null,
        ui_mode: "system",
      })
    : invoke("get_config");

export const getMonitoredProjects = (): Promise<MonitoredProject[]> =>
  PREVIEW
    ? Promise.resolve(
        previewEmpty() || previewGithub()
          ? []
          : previewStress()
            ? STRESS_MONITORED
            : FIXTURE_MONITORED,
      )
    : invoke("get_monitored_projects");

export const setMonitoredProjects = (
  accountId: string,
  projects: MonitoredProject[],
): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("set_monitored_projects", { accountId, projects });

export const setNotificationRules = (rules: NotificationRules): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("set_notification_rules", { rules });

export const setPollInterval = (secs: number): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("set_poll_interval", { secs });

export const setLocale = (code: string): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("set_locale", { code });

export const setUiMode = (mode: UiMode): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("set_ui_mode", { mode });

export const setLaunchAtLogin = (enabled: boolean): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("set_launch_at_login", { enabled });
