import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { act, fireEvent, screen } from "@testing-library/react";
import { listen } from "@tauri-apps/api/event";

import {
  appInfo,
  checkForUpdates,
  getConfig,
  getUpdateState,
  installUpdate,
  openUpdateRelease,
  setLaunchAtLogin,
  setLocale,
  setNotificationRules,
  setPollInterval,
  setUiMode,
} from "../api";
import type { Config, UpdateState } from "../types";
import { renderWithI18n, user } from "../test/utils";
import SettingsSection from "./SettingsSection";

vi.mock("../api", () => ({
  appInfo: vi.fn(),
  checkForUpdates: vi.fn(),
  getConfig: vi.fn(),
  getUpdateState: vi.fn(),
  installUpdate: vi.fn(),
  openUpdateRelease: vi.fn(),
  setLaunchAtLogin: vi.fn(),
  setLocale: vi.fn(),
  setNotificationRules: vi.fn(),
  setPollInterval: vi.fn(),
  setUiMode: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

// poll_interval_secs is deliberately 45 (not the component's default 30) so a passing
// `findByDisplayValue("45")` proves the on-mount getConfig load resolved before we interact.
const baseConfig: Config = {
  accounts: [],
  monitored: [],
  rules: {
    on_start: false,
    on_success: true,
    on_fail: true,
    on_cancel: false,
    job_on_start: false,
    job_on_success: false,
    job_on_fail: false,
    job_on_cancel: false,
  },
  poll_interval_secs: 45,
  launch_at_login: false,
  locale: null,
  ui_mode: "system",
};

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
    last_checked_at: "123",
    available: {
      version: "0.1.4",
      body: "Fixes",
      date: "2026-06-29T12:00:00Z",
      release_url: "https://github.com/Fuitad/cimon/releases/latest",
      self_updatable: selfUpdatable,
    },
  });

beforeEach(() => {
  vi.mocked(getConfig).mockResolvedValue(baseConfig);
  vi.mocked(getUpdateState).mockResolvedValue(updateState());
  vi.mocked(checkForUpdates).mockResolvedValue(updateState({ status: "up_to_date" }));
  vi.mocked(installUpdate).mockResolvedValue(updateState({ status: "installed" }));
  vi.mocked(openUpdateRelease).mockResolvedValue(undefined);
  vi.mocked(appInfo).mockResolvedValue({ version: "0.1.3", commit: "abc1234" });
  vi.mocked(setLaunchAtLogin).mockResolvedValue(undefined);
  vi.mocked(setLocale).mockResolvedValue(undefined);
  vi.mocked(setNotificationRules).mockResolvedValue(undefined);
  vi.mocked(setPollInterval).mockResolvedValue(undefined);
  vi.mocked(setUiMode).mockResolvedValue(undefined);
  vi.mocked(listen).mockResolvedValue(() => {});
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("SettingsSection", () => {
  it("persists a notification toggle as the merged rule set", async () => {
    renderWithI18n(<SettingsSection />);
    await screen.findByDisplayValue("45");

    await user().click(screen.getByRole("checkbox", { name: "settings.onStart" }));

    expect(setNotificationRules).toHaveBeenCalledWith({
      on_start: true,
      on_success: true,
      on_fail: true,
      on_cancel: false,
      job_on_start: false,
      job_on_success: false,
      job_on_fail: false,
      job_on_cancel: false,
    });
  });

  it("persists a job event toggle as the merged rule set", async () => {
    renderWithI18n(<SettingsSection />);
    await screen.findByDisplayValue("45");

    await user().click(screen.getByRole("checkbox", { name: "settings.jobOnFail" }));

    expect(setNotificationRules).toHaveBeenCalledWith({
      on_start: false,
      on_success: true,
      on_fail: true,
      on_cancel: false,
      job_on_start: false,
      job_on_success: false,
      job_on_fail: true,
      job_on_cancel: false,
    });
  });

  it("persists the pipeline cancel toggle as the merged rule set", async () => {
    renderWithI18n(<SettingsSection />);
    await screen.findByDisplayValue("45");

    await user().click(screen.getByRole("checkbox", { name: "settings.onCancel" }));

    expect(setNotificationRules).toHaveBeenCalledWith({
      on_start: false,
      on_success: true,
      on_fail: true,
      on_cancel: true,
      job_on_start: false,
      job_on_success: false,
      job_on_fail: false,
      job_on_cancel: false,
    });
  });

  it("clamps the poll interval to [10, 3600] on blur and persists the clamped value", async () => {
    renderWithI18n(<SettingsSection />);
    await screen.findByDisplayValue("45");
    const input = screen.getByRole("spinbutton");
    const u = user();

    await u.clear(input);
    await u.type(input, "5");
    await u.tab();
    expect(setPollInterval).toHaveBeenLastCalledWith(10);

    await u.clear(input);
    await u.type(input, "9999");
    await u.tab();
    expect(setPollInterval).toHaveBeenLastCalledWith(3600);
  });

  it("reverts the launch-at-login toggle and shows an error when the backend write fails", async () => {
    vi.mocked(setLaunchAtLogin).mockRejectedValueOnce(new Error("denied"));
    renderWithI18n(<SettingsSection />);
    await screen.findByDisplayValue("45");
    const toggle = screen.getByRole("checkbox", { name: "settings.launchAtLogin" });

    await user().click(toggle);

    expect(setLaunchAtLogin).toHaveBeenCalledWith(true);
    expect(await screen.findByRole("alert")).toHaveTextContent("settings.launchAtLoginError");
    expect(toggle).not.toBeChecked();
  });

  it("persists the chosen appearance", async () => {
    renderWithI18n(<SettingsSection />);
    await screen.findByDisplayValue("45");

    await user().selectOptions(
      screen.getByRole("combobox", { name: "settings.appearance" }),
      "dark",
    );

    expect(setUiMode).toHaveBeenCalledWith("dark");
  });

  it("persists the chosen locale from the language select", async () => {
    renderWithI18n(<SettingsSection />);
    await screen.findByDisplayValue("45");

    // The select is controlled to i18n.resolvedLanguage, so userEvent.selectOptions cannot drive it
    // reliably; fireEvent.change dispatches the onChange directly. onLanguage calls i18n.changeLanguage
    // AND setLocale with the same code, so asserting the backend persistence proves the select's
    // onChange path ran end-to-end with the chosen locale.
    fireEvent.change(screen.getByRole("combobox", { name: "settings.language" }), {
      target: { value: "fr" },
    });

    expect(setLocale).toHaveBeenCalledWith("fr");
  });

  it("checks for updates manually and shows the up-to-date state", async () => {
    renderWithI18n(<SettingsSection />);
    await screen.findByDisplayValue("45");

    await user().click(screen.getByRole("button", { name: "settings.checkForUpdates" }));

    expect(checkForUpdates).toHaveBeenCalled();
    expect(await screen.findByText("settings.updateUpToDate")).toBeInTheDocument();
  });

  it("installs an available self-updatable release from settings", async () => {
    vi.mocked(getUpdateState).mockResolvedValue(availableUpdate(true));
    renderWithI18n(<SettingsSection />);

    expect(await screen.findByText("settings.updateAvailable")).toBeInTheDocument();
    await user().click(screen.getByRole("button", { name: "settings.installRestart" }));

    expect(installUpdate).toHaveBeenCalled();
  });

  it("recovers the settings update row when install fails", async () => {
    vi.mocked(getUpdateState).mockResolvedValue(availableUpdate(true));
    vi.mocked(installUpdate).mockRejectedValueOnce(new Error("offline"));
    renderWithI18n(<SettingsSection />);

    expect(await screen.findByText("settings.updateAvailable")).toBeInTheDocument();
    await user().click(screen.getByRole("button", { name: "settings.installRestart" }));

    expect(await screen.findByText("settings.updateError")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "settings.installRestart" })).not.toBeDisabled();
  });

  it("opens the release page for a Linux update from settings", async () => {
    vi.mocked(getUpdateState).mockResolvedValue(availableUpdate(false));
    renderWithI18n(<SettingsSection />);

    expect(await screen.findByText("settings.updateAvailable")).toBeInTheDocument();
    await user().click(screen.getByRole("button", { name: "settings.openReleasePage" }));

    expect(openUpdateRelease).toHaveBeenCalled();
    expect(installUpdate).not.toHaveBeenCalled();
  });

  it("refreshes update state when the backend emits an update event", async () => {
    let updateCb: (() => void) | undefined;
    vi.mocked(listen).mockImplementation((event, handler) => {
      if (event === "update-state-updated") updateCb = handler as unknown as () => void;
      return Promise.resolve(() => {});
    });
    vi.mocked(getUpdateState)
      .mockResolvedValueOnce(updateState())
      .mockResolvedValueOnce(availableUpdate(true));
    renderWithI18n(<SettingsSection />);
    await screen.findByText("settings.updateIdle");
    expect(updateCb).toBeDefined();

    await act(async () => {
      updateCb!();
    });

    expect(await screen.findByText("settings.updateAvailable")).toBeInTheDocument();
  });
});
