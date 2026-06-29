import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { fireEvent, screen } from "@testing-library/react";

import {
  getConfig,
  setLaunchAtLogin,
  setLocale,
  setNotificationRules,
  setPollInterval,
  setUiMode,
} from "../api";
import type { Config } from "../types";
import { renderWithI18n, user } from "../test/utils";
import SettingsSection from "./SettingsSection";

vi.mock("../api", () => ({
  getConfig: vi.fn(),
  setLaunchAtLogin: vi.fn(),
  setLocale: vi.fn(),
  setNotificationRules: vi.fn(),
  setPollInterval: vi.fn(),
  setUiMode: vi.fn(),
}));

// poll_interval_secs is deliberately 45 (not the component's default 30) so a passing
// `findByDisplayValue("45")` proves the on-mount getConfig load resolved before we interact.
const baseConfig: Config = {
  accounts: [],
  monitored: [],
  rules: {
    on_start: false,
    on_success: true,
    on_fail: true,
    job_on_start: false,
    job_on_success: false,
    job_on_fail: false,
  },
  poll_interval_secs: 45,
  launch_at_login: false,
  locale: null,
  ui_mode: "system",
};

beforeEach(() => {
  vi.mocked(getConfig).mockResolvedValue(baseConfig);
  vi.mocked(setLaunchAtLogin).mockResolvedValue(undefined);
  vi.mocked(setLocale).mockResolvedValue(undefined);
  vi.mocked(setNotificationRules).mockResolvedValue(undefined);
  vi.mocked(setPollInterval).mockResolvedValue(undefined);
  vi.mocked(setUiMode).mockResolvedValue(undefined);
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
      job_on_start: false,
      job_on_success: false,
      job_on_fail: false,
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
      job_on_start: false,
      job_on_success: false,
      job_on_fail: true,
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
});
