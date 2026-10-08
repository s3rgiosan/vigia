// Shared builders for tests. All data is fabricated.

import type { AccountSnapshot, RepoSnapshot, RepoStatus, Run, Snapshot } from "../lib/snapshot";
import type { Account, Organization, Settings, SettingsView, WatchedRepo } from "../lib/tauri";

export function makeAccount(overrides: Partial<AccountSnapshot> = {}): AccountSnapshot {
  return {
    id: "acc-1",
    label: "Acme",
    kind: "github",
    auth_error: false,
    auth_reason: null,
    unreachable: false,
    rate_limited_until: null,
    effective_interval_secs: 60,
    configured_interval_secs: 60,
    keychain_denied: false,
    error: null,
    ...overrides,
  };
}

export function makeRun(overrides: Partial<Run> = {}): Run {
  return {
    id: 1,
    attempt: 1,
    state: "failed",
    branch: "main",
    group: "main",
    name: "CI",
    url: "https://example.com/run/1",
    updated_at: "2030-01-01T11:55:00Z",
    pull_request: false,
    fork: false,
    tag: false,
    ...overrides,
  };
}

export function makeRepo(
  name: string,
  status: RepoStatus,
  runs: Run[] = [],
  extra: Partial<RepoSnapshot["state"]> = {},
  accountId = "acc-1",
): RepoSnapshot {
  return {
    account_id: accountId,
    repo: {
      id: name.length * 7,
      full_name: name,
      web_url: `https://example.com/${name}`,
      default_branch: "main",
    },
    state: {
      status,
      representative: runs[0] ?? null,
      groups: runs,
      last_checked: null,
      stale: false,
      note: null,
      ...extra,
    },
  };
}

export function makeSnapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return {
    generated_at: "2030-01-01T11:58:00Z",
    paused: false,
    secrets_blocked: false,
    config_read_only: false,
    config_error: null,
    color: "gray",
    tooltip: "",
    accounts: [],
    repos: [],
    update: null,
    ...overrides,
  };
}

export function makeSettings(overrides: Partial<Settings> = {}): Settings {
  return {
    poll_interval_secs: 60,
    exclude_pull_requests: false,
    branch_patterns: [],
    ignored_workflows: [],
    include_tags: false,
    notify_failures: true,
    notify_recoveries: true,
    launch_at_login: false,
    check_for_updates: true,
    ...overrides,
  };
}

/** A configured account as Settings lists it. */
export function makeSettingsAccount(overrides: Partial<Account> = {}): Account {
  return {
    id: "acc-1",
    kind: "github",
    label: "Acme",
    login: "acme-bot",
    ...overrides,
  };
}

/** A watched repository on github.com, owned by the part of `fullName` before its last slash. */
export function makeWatchedRepo(fullName = "acme/widgets", overrides: Partial<WatchedRepo> = {}): WatchedRepo {
  return {
    account_id: "acc-1",
    host: "github.com",
    owner: fullName.slice(0, Math.max(0, fullName.lastIndexOf("/"))),
    repo: {
      id: fullName.length * 7,
      full_name: fullName,
      web_url: `https://example.com/${fullName}`,
      default_branch: "main",
    },
    ...overrides,
  };
}

/** An organization without overrides on github.com. */
export function makeOrganization(owner = "acme", overrides: Partial<Organization> = {}): Organization {
  return {
    host: "github.com",
    owner,
    filters: { branch_patterns: null, ignored_workflows: null, include_tags: null },
    repo_count: 1,
    ...overrides,
  };
}

export function makeView(overrides: Partial<SettingsView> = {}): SettingsView {
  return {
    accounts: [],
    organizations: [],
    repos: [],
    settings: makeSettings(),
    read_only: false,
    secrets_blocked: false,
    ...overrides,
  };
}
