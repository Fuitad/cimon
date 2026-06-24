// TypeScript mirrors of the Rust model (src-tauri/src/model.rs) and command errors.

export type ProviderKind = "gitlab" | "github";

export interface Identity {
  username: string;
  name: string | null;
  email: string | null;
}

export interface Account {
  id: string;
  label: string;
  provider: ProviderKind;
  base_url: string;
  identity: Identity;
}

export interface MonitoredProject {
  account_id: string;
  project_id: number;
  name: string;
  web_url: string;
  /** Provider-specific project address ("owner/repo") for GitHub; null/absent for GitLab. */
  remote_ref?: string | null;
}

export interface NotificationRules {
  on_start: boolean;
  on_success: boolean;
  on_fail: boolean;
  /** Notify on pipeline-level transitions. */
  pipeline_level: boolean;
  /** Notify on job-level transitions (individual jobs within a pipeline). */
  job_level: boolean;
}

/** Color theme for the app windows. `system` follows the OS appearance. Mirrors Rust `UiMode`. */
export type UiMode = "system" | "light" | "dark";

export interface Config {
  accounts: Account[];
  monitored: MonitoredProject[];
  rules: NotificationRules;
  poll_interval_secs: number;
  launch_at_login: boolean;
  locale: string | null;
  ui_mode: UiMode;
}

export interface DiscoveredProject {
  id: number;
  name: string;
  web_url: string;
  /** Owning group / namespace path (e.g. "acme/backend"); empty when the provider reports none. */
  group: string;
  /** Provider-specific project address ("owner/repo") for GitHub; null/absent for GitLab. */
  remote_ref?: string | null;
}

/** Normalized pipeline status, mirroring Rust `PipelineStatus` (serde snake_case). */
export type PipelineStatusKind =
  | "running"
  | "success"
  | "failed"
  | "canceled"
  | "skipped"
  | "pending"
  | "manual"
  | "other";

/** A monitored project joined with its latest status, for the tray popover panel. Mirrors the Rust
 *  `PanelProject` DTO returned by `get_project_statuses`. */
export interface PanelProject {
  account_id: string;
  account_label: string;
  provider: ProviderKind;
  base_url: string;
  project_id: number;
  name: string;
  web_url: string;
  /** `null` until the first poll observes this project (a neutral "checking" row). */
  status: PipelineStatusKind | null;
  branch: string;
  /** Latest pipeline `updated_at` (RFC3339), or `null` when never polled. Rendered relative. */
  updated_at: string | null;
  /** `true` when the most recent poll failed: status/branch are last-known, shown as offline. */
  stale: boolean;
}

/** App version and build identity, for confirming which build is running. Mirrors Rust `AppInfo`. */
export interface AppInfo {
  version: string;
  /** Build time as epoch milliseconds (the running binary's mtime), or `null` if unavailable. */
  built_at_ms: number | null;
}

export type CommandErrorKind =
  | "unauthorized"
  | "invalid_base_url"
  | "invalid_input"
  | "network"
  | "http"
  | "storage"
  | "not_found";

export interface CommandError {
  kind: CommandErrorKind;
  message: string;
}

/** Narrow an unknown thrown value to a CommandError (Tauri rejects with the serialized error). */
export function asCommandError(e: unknown): CommandError {
  if (typeof e === "object" && e !== null && "kind" in e && "message" in e) {
    return e as CommandError;
  }
  return { kind: "network", message: String(e) };
}
