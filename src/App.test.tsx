import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { screen, waitFor } from "@testing-library/react";

import { getConfig, getMonitoredProjects, getTokenHealth, listAccounts } from "./api";
import { applyUiMode } from "./theme";
import type { Config } from "./types";
import { renderWithI18n } from "./test/utils";
import App from "./App";

// App imports `./api` and `./theme`; the child sections import the same modules as `../api` / `../theme`,
// which resolve to the same files, so mocking here covers the whole tree.
vi.mock("./api", () => ({
  listAccounts: vi.fn(),
  getConfig: vi.fn(),
  getTokenHealth: vi.fn(),
  getMonitoredProjects: vi.fn(),
  listDiscoveredProjects: vi.fn(),
  setMonitoredProjects: vi.fn(),
  setNotificationRules: vi.fn(),
  setPollInterval: vi.fn(),
  setLaunchAtLogin: vi.fn(),
  setUiMode: vi.fn(),
  setLocale: vi.fn(),
  addAccount: vi.fn(),
  removeAccount: vi.fn(),
  updateAccountToken: vi.fn(),
}));

vi.mock("./theme", () => ({ applyUiMode: vi.fn() }));

const config: Config = {
  accounts: [],
  monitored: [],
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
  ui_mode: "dark",
};

beforeEach(() => {
  vi.mocked(listAccounts).mockResolvedValue([]);
  vi.mocked(getConfig).mockResolvedValue(config);
  vi.mocked(getTokenHealth).mockResolvedValue([]);
  vi.mocked(getMonitoredProjects).mockResolvedValue([]);
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("App", () => {
  it("loads accounts and config on mount and applies the configured UI mode", async () => {
    renderWithI18n(<App />);

    // Mock-interception guard: a mis-resolved mock would leave these as the real api and never record.
    await waitFor(() => expect(listAccounts).toHaveBeenCalled());
    expect(getConfig).toHaveBeenCalled();
    await waitFor(() => expect(applyUiMode).toHaveBeenCalledWith("dark"));
  });

  it("completes config adoption through the locale branch when a locale is set", async () => {
    vi.mocked(getConfig).mockResolvedValue({ ...config, locale: "fr", ui_mode: "light" });
    renderWithI18n(<App />);

    // The getConfig().then block runs `if (cfg.locale) i18n.changeLanguage(cfg.locale)` and then
    // applyUiMode(cfg.ui_mode). Asserting applyUiMode fired with the config's mode proves the whole
    // block executed past the locale branch without throwing. (The live i18n switch is not reliably
    // observable here due to a react-i18next provider instance-identity quirk.)
    await waitFor(() => expect(applyUiMode).toHaveBeenCalledWith("light"));
  });

  it("renders the three settings sections", async () => {
    renderWithI18n(<App />);

    expect(await screen.findByText("accounts.title")).toBeInTheDocument();
    expect(screen.getByText("projects.title")).toBeInTheDocument();
    expect(screen.getByText("settings.notifications")).toBeInTheDocument();
  });

  it("still renders when the initial api calls fail", async () => {
    vi.mocked(listAccounts).mockRejectedValue(new Error("offline"));
    vi.mocked(getConfig).mockRejectedValue(new Error("offline"));
    renderWithI18n(<App />);

    expect(await screen.findByText("app.name")).toBeInTheDocument();
    expect(screen.getByText("accounts.title")).toBeInTheDocument();
  });
});
