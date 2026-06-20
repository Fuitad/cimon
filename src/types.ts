// TypeScript mirrors of the Rust model (src-tauri/src/model.rs) and command errors.

export type ProviderKind = "gitlab";

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

export interface Config {
  accounts: Account[];
  monitored: MonitoredProject[];
  rules: NotificationRules;
  poll_interval_secs: number;
  launch_at_login: boolean;
  locale: string | null;
}

export interface DiscoveredProject {
  id: number;
  name: string;
  web_url: string;
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
