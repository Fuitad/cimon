import { invoke } from "@tauri-apps/api/core";

import type {
  Account,
  AppInfo,
  Config,
  DiscoveredProject,
  Identity,
  MonitoredProject,
  NotificationRules,
  PanelProject,
  ProviderKind,
  UiMode,
} from "./types";

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
// never-polled) so every row treatment is visible in a plain browser.
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
  },
];

// `?preview=offline` reproduces a VPN-down scenario: projects that have never polled successfully
// (status null + stale) read as "can't connect", distinct from a freshly-added project still being
// polled for the first time (status null, not stale) which reads "checking".
const PANEL_OFFLINE_FIXTURE: PanelProject[] = [
  { ...PANEL_FIXTURE[2], status: null, branch: "", updated_at: null, stale: true },
  { ...PANEL_FIXTURE[1], status: null, branch: "", updated_at: null, stale: true },
  PANEL_FIXTURE[5], // never polled yet (status null, not stale) -> "checking"
];

export const getProjectStatuses = (): Promise<PanelProject[]> => {
  if (!PREVIEW) return invoke("get_project_statuses");
  if (previewEmpty()) return Promise.resolve([]);
  if (previewParam() === "multi") return Promise.resolve(PANEL_MULTI_FIXTURE);
  if (previewParam() === "offline") return Promise.resolve(PANEL_OFFLINE_FIXTURE);
  return Promise.resolve(PANEL_FIXTURE);
};

export const openProjectUrl = (url: string): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("open_project_url", { url });

export const appInfo = (): Promise<AppInfo> =>
  PREVIEW ? Promise.resolve({ version: "dev", built_at_ms: Date.now() }) : invoke("app_info");

export const showSettingsWindow = (): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("show_settings_window");

export const quitApp = (): Promise<void> => (PREVIEW ? Promise.resolve() : invoke("quit_app"));

export const hidePanel = (): Promise<void> => (PREVIEW ? Promise.resolve() : invoke("hide_panel"));

export const setPanelHeight = (height: number): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("set_panel_height", { height });

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
  return Promise.resolve([FIXTURE_ACCOUNT]);
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
        rules: {
          on_start: false,
          on_success: true,
          on_fail: true,
          pipeline_level: true,
          job_level: false,
        },
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
