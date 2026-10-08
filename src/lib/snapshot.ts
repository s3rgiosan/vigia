// Mirrors the Rust `Snapshot` emitted on `snapshot-updated`.

export type RunState = "failed" | "running" | "queued" | "success" | "canceled" | "neutral";
export type RepoStatus = "failed" | "error" | "running" | "success" | "none";
export type TrayColor = "red" | "orange" | "yellow" | "green" | "gray";
export type AccountKind = "github" | "gitlab";

export interface Run {
  id: number;
  attempt: number;
  state: RunState;
  branch: string;
  group: string;
  name: string;
  url: string;
  updated_at: string;
  pull_request: boolean;
  fork: boolean;
  tag: boolean;
}

export interface RepoInfo {
  id: number;
  full_name: string;
  web_url: string;
  default_branch: string;
}

export interface RepoState {
  status: RepoStatus;
  representative: Run | null;
  groups: Run[];
  last_checked: string | null;
  stale: boolean;
  note: string | null;
}

/** Why an account is in an auth error. */
export type AuthReason = "rejected" | "missing_token" | "keychain";

export interface AccountSnapshot {
  id: string;
  label: string;
  kind: AccountKind;
  auth_error: boolean;
  /** The cause of `auth_error`; null without one. */
  auth_reason: AuthReason | null;
  unreachable: boolean;
  rate_limited_until: number | null;
  effective_interval_secs: number;
  /** The interval the user chose; the effective one is longer while rate limits stretch it. */
  configured_interval_secs: number;
  /** The Keychain refused access to this account's token. */
  keychain_denied: boolean;
  error: string | null;
}

export interface RepoSnapshot {
  account_id: string;
  repo: RepoInfo;
  state: RepoState;
}

/** A newer release that can be installed. */
export interface UpdateInfo {
  version: string;
  notes: string | null;
}

export interface Snapshot {
  generated_at: string;
  paused: boolean;
  secrets_blocked: boolean;
  config_read_only: boolean;
  /** Why the config file could not be read; the app then runs read-only. */
  config_error: string | null;
  color: TrayColor;
  tooltip: string;
  accounts: AccountSnapshot[];
  repos: RepoSnapshot[];
  /** The release waiting to be installed, if one was found. */
  update: UpdateInfo | null;
}

export const SNAPSHOT_EVENT = "snapshot-updated";
