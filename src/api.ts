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

export const addAccount = (label: string, baseUrl: string, token: string): Promise<Identity> =>
  invoke("add_account", { label, baseUrl, token });

export const removeAccount = (id: string): Promise<void> => invoke("remove_account", { id });

export const listAccounts = (): Promise<Account[]> => invoke("list_accounts");

export const listDiscoveredProjects = (accountId: string): Promise<DiscoveredProject[]> =>
  invoke("list_discovered_projects", { accountId });

export const getConfig = (): Promise<Config> => invoke("get_config");

export const getMonitoredProjects = (): Promise<MonitoredProject[]> =>
  invoke("get_monitored_projects");

export const setMonitoredProjects = (
  accountId: string,
  projects: MonitoredProject[],
): Promise<void> => invoke("set_monitored_projects", { accountId, projects });

export const setNotificationRules = (rules: NotificationRules): Promise<void> =>
  invoke("set_notification_rules", { rules });

export const setPollInterval = (secs: number): Promise<void> =>
  invoke("set_poll_interval", { secs });

export const setLocale = (code: string): Promise<void> => invoke("set_locale", { code });

export const setLaunchAtLogin = (enabled: boolean): Promise<void> =>
  invoke("set_launch_at_login", { enabled });
