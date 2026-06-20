import { invoke } from "@tauri-apps/api/core";

import type {
  Account,
  Config,
  DiscoveredProject,
  Identity,
  MonitoredProject,
  NotificationRules,
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

export const addAccount = (label: string, baseUrl: string, token: string): Promise<Identity> =>
  PREVIEW
    ? Promise.resolve(FIXTURE_ACCOUNT.identity)
    : invoke("add_account", { label, baseUrl, token });

export const removeAccount = (id: string): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("remove_account", { id });

export const listAccounts = (): Promise<Account[]> => {
  if (!PREVIEW) return invoke("list_accounts");
  if (previewEmpty()) return Promise.resolve([]);
  if (previewStress()) return Promise.resolve(STRESS_ACCOUNTS);
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
  return Promise.resolve(FIXTURE_PROJECTS);
};

export const getConfig = (): Promise<Config> =>
  PREVIEW
    ? Promise.resolve({
        accounts: previewEmpty() ? [] : previewStress() ? STRESS_ACCOUNTS : [FIXTURE_ACCOUNT],
        monitored: previewEmpty() ? [] : previewStress() ? STRESS_MONITORED : FIXTURE_MONITORED,
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
      })
    : invoke("get_config");

export const getMonitoredProjects = (): Promise<MonitoredProject[]> =>
  PREVIEW
    ? Promise.resolve(previewEmpty() ? [] : previewStress() ? STRESS_MONITORED : FIXTURE_MONITORED)
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

export const setLaunchAtLogin = (enabled: boolean): Promise<void> =>
  PREVIEW ? Promise.resolve() : invoke("set_launch_at_login", { enabled });
