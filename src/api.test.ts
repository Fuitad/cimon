import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

import { withTauri, withoutTauri } from "./test/utils";
import type { MonitoredProject, NotificationRules } from "./types";

// `invoke` is replaced so the production-path tests can assert the Tauri command name + args without
// a real shell. The factory re-runs after each `vi.resetModules()`, yielding a fresh mock that the
// dynamically-imported api module then binds to.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

type Api = typeof import("./api");

const DEFAULT_RULES: NotificationRules = {
  on_start: false,
  on_success: true,
  on_fail: true,
  pipeline_level: true,
  job_level: false,
};

describe("api.ts invoke contract (inside the Tauri shell)", () => {
  let api: Api;
  let invoke: Mock;

  beforeEach(async () => {
    // PREVIEW is a module-level const evaluated at import, so the Tauri global MUST be present before
    // the module is (re)loaded. Reset the registry, then grab the fresh invoke and re-import api so
    // both bind to the same mocked core module.
    withTauri();
    vi.resetModules();
    const core = await import("@tauri-apps/api/core");
    invoke = vi.mocked(core.invoke);
    invoke.mockResolvedValue(undefined);
    api = await import("./api");
  });

  afterEach(() => {
    withoutTauri();
    vi.resetModules();
    vi.clearAllMocks();
  });

  const noArgCases: Array<[string, (a: Api) => Promise<unknown>, string]> = [
    ["getProjectStatuses", (a) => a.getProjectStatuses(), "get_project_statuses"],
    ["appInfo", (a) => a.appInfo(), "app_info"],
    ["showSettingsWindow", (a) => a.showSettingsWindow(), "show_settings_window"],
    ["quitApp", (a) => a.quitApp(), "quit_app"],
    ["hidePanel", (a) => a.hidePanel(), "hide_panel"],
    ["listAccounts", (a) => a.listAccounts(), "list_accounts"],
    ["getTokenHealth", (a) => a.getTokenHealth(), "get_token_health"],
    ["getConfig", (a) => a.getConfig(), "get_config"],
    ["getMonitoredProjects", (a) => a.getMonitoredProjects(), "get_monitored_projects"],
  ];

  it.each(noArgCases)("%s -> invoke(%s)", async (_name, call, command) => {
    await call(api);
    expect(invoke).toHaveBeenCalledWith(command);
  });

  const argCases: Array<[string, (a: Api) => Promise<unknown>, string, Record<string, unknown>]> = [
    [
      "openProjectUrl",
      (a) => a.openProjectUrl("https://x/p"),
      "open_project_url",
      { url: "https://x/p" },
    ],
    ["setPanelHeight", (a) => a.setPanelHeight(120), "set_panel_height", { height: 120 }],
    ["removeAccount", (a) => a.removeAccount("acc-1"), "remove_account", { id: "acc-1" }],
    [
      "addAccount",
      (a) => a.addAccount("github", "Work", "https://github.com", "tok"),
      "add_account",
      { provider: "github", label: "Work", baseUrl: "https://github.com", token: "tok" },
    ],
    [
      "updateAccountToken",
      (a) => a.updateAccountToken("acc-1", "newtok"),
      "update_account_token",
      { accountId: "acc-1", token: "newtok" },
    ],
    [
      "listDiscoveredProjects",
      (a) => a.listDiscoveredProjects("acc-1"),
      "list_discovered_projects",
      { accountId: "acc-1" },
    ],
    [
      "setMonitoredProjects",
      (a) => a.setMonitoredProjects("acc-1", [] as MonitoredProject[]),
      "set_monitored_projects",
      { accountId: "acc-1", projects: [] },
    ],
    [
      "setNotificationRules",
      (a) => a.setNotificationRules(DEFAULT_RULES),
      "set_notification_rules",
      { rules: DEFAULT_RULES },
    ],
    ["setPollInterval", (a) => a.setPollInterval(45), "set_poll_interval", { secs: 45 }],
    ["setLocale", (a) => a.setLocale("fr"), "set_locale", { code: "fr" }],
    ["setUiMode", (a) => a.setUiMode("dark"), "set_ui_mode", { mode: "dark" }],
    ["setLaunchAtLogin", (a) => a.setLaunchAtLogin(true), "set_launch_at_login", { enabled: true }],
  ];

  it.each(argCases)("%s -> invoke(%s, args)", async (_name, call, command, args) => {
    await call(api);
    expect(invoke).toHaveBeenCalledWith(command, args);
  });
});

describe("api.ts dev preview fixtures (outside the Tauri shell)", () => {
  const loadPreview = async (param: string | null): Promise<Api> => {
    withoutTauri();
    window.history.replaceState({}, "", param ? `/?preview=${param}` : "/");
    vi.resetModules();
    return import("./api");
  };

  afterEach(() => {
    window.history.replaceState({}, "", "/");
    vi.resetModules();
  });

  it("listAccounts returns the default single GitLab fixture", async () => {
    const api = await loadPreview(null);
    const accounts = await api.listAccounts();
    expect(accounts).toHaveLength(1);
    expect(accounts[0]).toMatchObject({ id: "acc-1", provider: "gitlab" });
  });

  it("listAccounts returns [] for ?preview=empty", async () => {
    const api = await loadPreview("empty");
    expect(await api.listAccounts()).toEqual([]);
  });

  it("listAccounts returns two accounts for ?preview=stress", async () => {
    const api = await loadPreview("stress");
    expect(await api.listAccounts()).toHaveLength(2);
  });

  it("listAccounts returns a GitHub account for ?preview=github", async () => {
    const api = await loadPreview("github");
    const accounts = await api.listAccounts();
    expect(accounts).toHaveLength(1);
    expect(accounts[0].provider).toBe("github");
  });

  it("listAccounts returns the token-health accounts for ?preview=tokenhealth", async () => {
    const api = await loadPreview("tokenhealth");
    const ids = (await api.listAccounts()).map((a) => a.id);
    expect(ids).toContain("th-dead");
  });

  it("getProjectStatuses returns [] for ?preview=empty", async () => {
    const api = await loadPreview("empty");
    expect(await api.getProjectStatuses()).toEqual([]);
  });

  it("getProjectStatuses returns the default panel fixture with a failed row", async () => {
    const api = await loadPreview(null);
    const statuses = await api.getProjectStatuses();
    expect(statuses.length).toBeGreaterThan(0);
    expect(statuses.some((p) => p.status === "failed")).toBe(true);
  });

  it("getConfig returns the default config (one account, 30s interval, system theme)", async () => {
    const api = await loadPreview(null);
    const cfg = await api.getConfig();
    expect(cfg.accounts).toHaveLength(1);
    expect(cfg.poll_interval_secs).toBe(30);
    expect(cfg.ui_mode).toBe("system");
  });

  it("getConfig returns empty accounts and monitored for ?preview=empty", async () => {
    const api = await loadPreview("empty");
    const cfg = await api.getConfig();
    expect(cfg.accounts).toEqual([]);
    expect(cfg.monitored).toEqual([]);
  });

  it("getConfig returns the stress accounts for ?preview=stress", async () => {
    const api = await loadPreview("stress");
    expect((await api.getConfig()).accounts).toHaveLength(2);
  });

  it("getConfig returns a GitHub account and nothing monitored for ?preview=github", async () => {
    const api = await loadPreview("github");
    const cfg = await api.getConfig();
    expect(cfg.accounts).toHaveLength(1);
    expect(cfg.accounts[0].provider).toBe("github");
    expect(cfg.monitored).toEqual([]);
  });

  it("getProjectStatuses returns a GitHub row for ?preview=multi", async () => {
    const api = await loadPreview("multi");
    const statuses = await api.getProjectStatuses();
    expect(statuses.some((p) => p.provider === "github")).toBe(true);
  });

  it("getProjectStatuses returns stale (offline) rows for ?preview=offline", async () => {
    const api = await loadPreview("offline");
    const statuses = await api.getProjectStatuses();
    expect(statuses.length).toBeGreaterThan(0);
    expect(statuses.some((p) => p.stale)).toBe(true);
  });

  it("getProjectStatuses returns an auth-failed row for ?preview=tokenhealth", async () => {
    const api = await loadPreview("tokenhealth");
    const statuses = await api.getProjectStatuses();
    expect(statuses.some((p) => p.auth_failed)).toBe(true);
  });

  it("listDiscoveredProjects rejects for the failing stress account (acc-2)", async () => {
    const api = await loadPreview("stress");
    await expect(api.listDiscoveredProjects("acc-2")).rejects.toThrow();
  });

  it("listDiscoveredProjects resolves projects for a healthy stress account", async () => {
    const api = await loadPreview("stress");
    expect((await api.listDiscoveredProjects("acc-1")).length).toBeGreaterThan(0);
  });
});
