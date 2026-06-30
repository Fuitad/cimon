import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { act, screen } from "@testing-library/react";
import { listen } from "@tauri-apps/api/event";

import {
  appInfo,
  dismissUpdate,
  getConfig,
  getProjectStatuses,
  getUpdateState,
  installUpdate,
  openUpdateRelease,
  openProjectUrl,
  quitApp,
  showSettingsWindow,
} from "./api";
import type { Config, PanelProject, UpdateState } from "./types";
import { renderWithI18n, user } from "./test/utils";
import Panel from "./Panel";

vi.mock("./api", () => ({
  appInfo: vi.fn(),
  dismissUpdate: vi.fn(),
  getConfig: vi.fn(),
  getProjectStatuses: vi.fn(),
  getUpdateState: vi.fn(),
  installUpdate: vi.fn(),
  hidePanel: vi.fn(),
  openUpdateRelease: vi.fn(),
  openProjectUrl: vi.fn(),
  quitApp: vi.fn(),
  setPanelHeight: vi.fn(),
  showSettingsWindow: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

const row = (over: Partial<PanelProject>): PanelProject => ({
  account_id: "acc-1",
  account_label: "Work",
  provider: "gitlab",
  base_url: "https://gitlab.com",
  project_id: 1,
  name: "web-app",
  web_url: "https://gl/web-app",
  status: "success",
  branch: "main",
  updated_at: null,
  stale: false,
  auth_failed: false,
  ...over,
});

const config = (accountCount: number): Config => ({
  accounts: Array.from({ length: accountCount }, (_, i) => ({
    id: `acc-${i + 1}`,
    label: "Work",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    identity: { username: "dev", name: null, email: null },
  })),
  monitored: [],
  rules: {
    on_start: false,
    on_success: true,
    on_fail: true,
    job_on_start: false,
    job_on_success: false,
    job_on_fail: false,
  },
  poll_interval_secs: 30,
  launch_at_login: false,
  locale: null,
  ui_mode: "system",
});

const updateState = (over: Partial<UpdateState> = {}): UpdateState => ({
  status: "idle",
  available: null,
  last_checked_at: null,
  error: null,
  progress: null,
  dismissed_version: null,
  ...over,
});

const availableUpdate = (selfUpdatable = true): UpdateState =>
  updateState({
    status: "available",
    available: {
      version: "0.1.4",
      body: "Fixes",
      date: "2026-06-29T12:00:00Z",
      release_url: "https://github.com/Fuitad/cimon/releases/latest",
      self_updatable: selfUpdatable,
    },
  });

beforeEach(() => {
  vi.mocked(getProjectStatuses).mockResolvedValue([]);
  vi.mocked(getConfig).mockResolvedValue(config(1));
  vi.mocked(getUpdateState).mockResolvedValue(updateState());
  vi.mocked(appInfo).mockResolvedValue({ version: "0.1.0", commit: "abc1234" });
  vi.mocked(installUpdate).mockResolvedValue(updateState({ status: "installed" }));
  vi.mocked(dismissUpdate).mockResolvedValue(updateState());
  vi.mocked(openUpdateRelease).mockResolvedValue(undefined);
  vi.mocked(listen).mockResolvedValue(() => {});
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("Panel", () => {
  it("summarizes a failing project with precedence over running/success", async () => {
    vi.mocked(getProjectStatuses).mockResolvedValue([
      row({ project_id: 1, name: "web-app", status: "failed" }),
      row({ project_id: 2, name: "api", status: "running" }),
    ]);
    renderWithI18n(<Panel />);

    expect(await screen.findByText("panel.summaryFailing")).toBeInTheDocument();
    expect(screen.queryByText("panel.summaryRunning")).toBeNull();
  });

  it("reports running over a single unreachable project in the header", async () => {
    vi.mocked(getProjectStatuses).mockResolvedValue([
      row({ project_id: 1, name: "api", status: "running" }),
      // Never polled successfully and currently failing -> "unreachable".
      row({ project_id: 2, name: "web", status: null, stale: true }),
    ]);
    renderWithI18n(<Panel />);

    expect(await screen.findByText("panel.summaryRunning")).toBeInTheDocument();
    expect(screen.queryByText("panel.summaryUnreachable")).toBeNull();
  });

  it("summarizes all-passing when every project succeeds", async () => {
    vi.mocked(getProjectStatuses).mockResolvedValue([
      row({ project_id: 1, status: "success" }),
      row({ project_id: 2, name: "api", status: "success" }),
    ]);
    renderWithI18n(<Panel />);

    expect(await screen.findByText("panel.summaryAllPassing")).toBeInTheDocument();
  });

  it("renders the no-accounts empty state and opens settings from its CTA", async () => {
    vi.mocked(getProjectStatuses).mockResolvedValue([]);
    vi.mocked(getConfig).mockResolvedValue(config(0));
    renderWithI18n(<Panel />);

    expect(await screen.findByText("panel.emptyNoAccountsTitle")).toBeInTheDocument();
    await user().click(screen.getByRole("button", { name: "panel.emptyNoAccountsCta" }));
    expect(showSettingsWindow).toHaveBeenCalled();
  });

  it("renders the no-projects empty state when accounts exist", async () => {
    vi.mocked(getProjectStatuses).mockResolvedValue([]);
    vi.mocked(getConfig).mockResolvedValue(config(2));
    renderWithI18n(<Panel />);

    expect(await screen.findByText("panel.emptyNoProjectsTitle")).toBeInTheDocument();
  });

  it("opens a project's URL when its row is clicked", async () => {
    vi.mocked(getProjectStatuses).mockResolvedValue([row({ web_url: "https://gl/web-app" })]);
    renderWithI18n(<Panel />);

    await user().click(await screen.findByRole("button", { name: /web-app/ }));

    expect(openProjectUrl).toHaveBeenCalledWith("acc-1", 1);
  });

  it("invokes the footer settings and quit actions", async () => {
    vi.mocked(getProjectStatuses).mockResolvedValue([row({})]);
    renderWithI18n(<Panel />);
    const u = user();
    await screen.findByRole("button", { name: /web-app/ });

    await u.click(screen.getByRole("button", { name: "panel.settings" }));
    await u.click(screen.getByRole("button", { name: "panel.quit" }));

    expect(showSettingsWindow).toHaveBeenCalled();
    expect(quitApp).toHaveBeenCalled();
  });

  it("renders a relative-time label for a recently updated project", async () => {
    const twoHoursAgo = new Date(Date.now() - 2 * 60 * 60 * 1000).toISOString();
    vi.mocked(getProjectStatuses).mockResolvedValue([row({ updated_at: twoHoursAgo })]);
    renderWithI18n(<Panel />);

    expect(await screen.findByText("panel.hourAgo")).toBeInTheDocument();
  });

  it("re-fetches and re-renders when a status-updated event fires", async () => {
    let statusCb: (() => void) | undefined;
    vi.mocked(listen).mockImplementation((event, handler) => {
      if (event === "status-updated") statusCb = handler as unknown as () => void;
      return Promise.resolve(() => {});
    });
    vi.mocked(getProjectStatuses).mockResolvedValue([row({ status: "success" })]);
    renderWithI18n(<Panel />);
    await screen.findByText("panel.summaryAllPassing");
    expect(statusCb).toBeDefined();

    // The poller emits "status-updated"; firing the captured callback must refresh the snapshot.
    vi.mocked(getProjectStatuses).mockResolvedValue([row({ status: "failed" })]);
    await act(async () => {
      statusCb!();
    });

    expect(await screen.findByText("panel.summaryFailing")).toBeInTheDocument();
  });

  it("renders an available update banner and installs from the primary action", async () => {
    vi.mocked(getProjectStatuses).mockResolvedValue([row({})]);
    vi.mocked(getUpdateState).mockResolvedValue(availableUpdate(true));
    renderWithI18n(<Panel />);

    expect(await screen.findByText("panel.updateAvailable")).toBeInTheDocument();
    await user().click(screen.getByRole("button", { name: "panel.installRestart" }));

    expect(installUpdate).toHaveBeenCalled();
  });

  it("renders Linux update fallback without install progress", async () => {
    vi.mocked(getProjectStatuses).mockResolvedValue([row({})]);
    vi.mocked(getUpdateState).mockResolvedValue(availableUpdate(false));
    renderWithI18n(<Panel />);

    expect(await screen.findByText("panel.updateAvailable")).toBeInTheDocument();
    await user().click(screen.getByRole("button", { name: "panel.openReleasePage" }));

    expect(openUpdateRelease).toHaveBeenCalled();
    expect(installUpdate).not.toHaveBeenCalled();
  });

  it("surfaces an error in the banner when an install fails", async () => {
    vi.mocked(getProjectStatuses).mockResolvedValue([row({})]);
    vi.mocked(getUpdateState).mockResolvedValue(availableUpdate(true));
    vi.mocked(installUpdate).mockRejectedValueOnce(new Error("download failed"));
    renderWithI18n(<Panel />);

    await user().click(await screen.findByRole("button", { name: "panel.installRestart" }));

    expect(await screen.findByText("panel.updateFailed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "panel.installRestart" })).not.toBeDisabled();
  });
});
