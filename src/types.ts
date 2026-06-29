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
  /** Notify when a pipeline starts. */
  on_start: boolean;
  /** Notify when a pipeline succeeds. */
  on_success: boolean;
  /** Notify when a pipeline fails. */
  on_fail: boolean;
  /** Notify when an individual job starts. */
  job_on_start: boolean;
  /** Notify when an individual job succeeds. */
  job_on_success: boolean;
  /** Notify when an individual job fails. */
  job_on_fail: boolean;
}

/** Quiet-ish default: pipeline completion events on, every job event opt-in. Single source of
 * truth shared by the settings UI's initial state and the dev-preview fixture. */
export const DEFAULT_NOTIFICATION_RULES: NotificationRules = {
  on_start: false,
  on_success: true,
  on_fail: true,
  job_on_start: false,
  job_on_success: false,
  job_on_fail: false,
};

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
  /** `true` when the account's token is dead (expired/revoked/invalid). Takes visual precedence
   *  over `stale`: the row reads "authentication failed", not "offline". */
  auth_failed: boolean;
}

/** Per-account token health for the settings UI. Mirrors the Rust `AccountTokenHealth` DTO returned
 *  by `get_token_health`. */
export interface AccountTokenHealth {
  account_id: string;
  auth_failed: boolean;
  /** Raw provider expiry string (e.g. "2026-08-15" or "2026-08-15 14:23:01 UTC"), or `null` when
   *  the token has no expiry / has not been checked yet. Presence drives whether an expiry line is
   *  shown; `expires_in_days` drives what it says. */
  expires_at: string | null;
  /** Whole UTC days until expiry (negative once past, 0 on the expiry day), computed by the backend.
   *  `null` when there is no expiry OR the provider string could not be parsed. The frontend renders
   *  this value instead of re-parsing the date itself. */
  expires_in_days: number | null;
}

/** App version and build identity, for confirming which build is running. Mirrors Rust `AppInfo`. */
export interface AppInfo {
  version: string;
  /** Short commit SHA the running binary was built from, or `null` when built outside a git checkout. */
  commit: string | null;
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
