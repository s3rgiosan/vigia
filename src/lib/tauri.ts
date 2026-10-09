// Typed wrappers around the Tauri commands and events the windows use.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isSettingsView, isSnapshot } from "./guards";
import { SNAPSHOT_EVENT, type AccountKind, type RepoInfo, type RepoOrder, type Snapshot, type UpdateInfo } from "./snapshot";

export const SETTINGS_PANE_EVENT = "settings-pane";

function invalidPayload(what: string): { kind: "internal"; message: string } {
  return { kind: "internal", message: `Received an unexpected ${what} from the backend.` };
}

/** An organization's filter overrides. Null inherits the global setting; an empty list means "nothing". */
export interface OrgFilters {
  branch_patterns: string[] | null;
  ignored_workflows: string[] | null;
  include_tags: boolean | null;
}

/** An owner (organization, user or group) with at least one watched repository. */
export interface Organization {
  host: string;
  owner: string;
  filters: OrgFilters;
  repo_count: number;
}

export interface Account {
  id: string;
  kind: AccountKind;
  label: string;
  base_url?: string;
  user_id?: number;
  login?: string;
}

export interface WatchedRepo {
  account_id: string;
  repo: RepoInfo;
  /** The server the repo lives on, such as github.com. */
  host: string;
  /** The organization, user or group that owns the repo. */
  owner: string;
  /** Branch globs for this repo. Absent or null inherits the organization or global list; empty watches the default branch. */
  branch_patterns?: string[] | null;
  /** Workflow name globs hidden for this repo. Absent or null inherits; empty ignores nothing. */
  ignored_workflows?: string[] | null;
  /** Whether tag runs count for this repo. Absent inherits the organization or global setting. */
  include_tags?: boolean | null;
}

export interface Settings {
  poll_interval_secs: number;
  exclude_pull_requests: boolean;
  branch_patterns: string[];
  /** Case-insensitive globs of workflow names to ignore. Empty ignores nothing. */
  ignored_workflows: string[];
  /** Whether the latest run of each tag-triggered workflow counts. */
  include_tags: boolean;
  notify_failures: boolean;
  notify_recoveries: boolean;
  launch_at_login: boolean;
  /** Whether Vigia looks for a new release a minute after launch and daily. */
  check_for_updates: boolean;
  repo_order: RepoOrder;
  /** Whether the popup lists repositories under account and organization headings. */
  group_by_org: boolean;
}

export interface SettingsView {
  accounts: Account[];
  /** Owners of watched repositories, sorted by host, then owner. */
  organizations: Organization[];
  repos: WatchedRepo[];
  settings: Settings;
  read_only: boolean;
  secrets_blocked: boolean;
}

export interface NewAccount {
  kind: AccountKind;
  label: string;
  base_url?: string;
  token: string;
  allow_insecure: boolean;
}

export interface AccountIdentity {
  user_id: number;
  login: string;
}

export interface PickerRepo {
  repo: RepoInfo;
  watched: boolean;
  watched_via: string | null;
}

export async function getSnapshot(): Promise<Snapshot> {
  const snapshot = await invoke<unknown>("get_snapshot");
  if (!isSnapshot(snapshot)) {
    throw invalidPayload("snapshot");
  }
  return snapshot;
}

export function refreshNow(): Promise<void> {
  return invoke("refresh_now");
}

export function setPaused(paused: boolean): Promise<void> {
  return invoke("set_paused", { paused });
}

export function openUrl(accountId: string, url: string): Promise<void> {
  return invoke("open_url", { accountId, url });
}

export function hidePopup(): Promise<void> {
  return invoke("hide_popup");
}

/** Opens GitHub's fine-grained token form, prefilled with the grants Vigia needs. */
export function openGithubTokenPage(owner: string | null): Promise<void> {
  return invoke("open_github_token_page", { owner });
}

/** Highlights the pane in the Settings window's native toolbar. */
export function selectSettingsPane(pane: string): Promise<void> {
  return invoke("select_settings_pane", { pane });
}

/** Shows or hides the native toolbar of the Settings window. */
export function setSettingsToolbarEnabled(enabled: boolean): Promise<void> {
  return invoke("set_settings_toolbar_enabled", { enabled });
}

export type SettingsPane = "accounts" | "repos" | "branches" | "general";
export type SettingsSheet = "add" | "replace";

export interface SettingsTarget {
  pane?: SettingsPane;
  sheet?: SettingsSheet;
  /** The account a "replace" sheet applies to. */
  accountId?: string;
}

/** Opens the Settings window, optionally on a pane and with a sheet already showing. */
export function openSettings(target?: SettingsTarget): Promise<void> {
  return invoke("open_settings", {
    pane: target?.pane ?? null,
    sheet: target?.sheet ?? null,
    accountId: target?.accountId ?? null,
  });
}

export interface SettingsOpen {
  pane: string | null;
  sheet: string | null;
  account_id: string | null;
}

/** @deprecated Use `SettingsOpen`. */
export type SettingsOpenPayload = SettingsOpen;

/** Fires when an already open Settings window is asked to show a pane or sheet. */
export function onSettingsOpen(handler: (payload: SettingsOpen) => void): Promise<UnlistenFn> {
  return listen<SettingsOpen>("settings-open", (event) => handler(event.payload));
}

/** Fires when the native toolbar selects a pane. */
export function onSettingsPane(handler: (pane: string) => void): Promise<UnlistenFn> {
  return listen<string>(SETTINGS_PANE_EVENT, (event) => handler(event.payload));
}

export function onSnapshot(handler: (snapshot: Snapshot) => void): Promise<UnlistenFn> {
  return listen<unknown>(SNAPSHOT_EVENT, (event) => {
    if (!isSnapshot(event.payload)) {
      console.error("ignored a malformed snapshot event");
      return;
    }
    handler(event.payload);
  });
}

export async function getSettings(): Promise<SettingsView> {
  const view = await invoke<unknown>("get_settings");
  if (!isSettingsView(view)) {
    throw invalidPayload("settings payload");
  }
  return view;
}

export function testConnection(input: NewAccount): Promise<AccountIdentity> {
  return invoke<AccountIdentity>("test_connection", { input });
}

export function addAccount(input: NewAccount): Promise<Account> {
  return invoke<Account>("add_account", { input });
}

export function renameAccount(accountId: string, label: string): Promise<void> {
  return invoke("rename_account", { accountId, label });
}

export function replaceToken(accountId: string, token: string): Promise<void> {
  return invoke("replace_token", { accountId, token });
}

export function deleteAccount(accountId: string): Promise<void> {
  return invoke("delete_account", { accountId });
}

export function deleteAllAccounts(): Promise<number> {
  return invoke<number>("delete_all_accounts");
}

export function listPickerRepos(accountId: string, refresh: boolean): Promise<PickerRepo[]> {
  return invoke<PickerRepo[]>("list_picker_repos", { accountId, refresh });
}

export function setWatched(accountId: string, repoIds: number[]): Promise<number> {
  return invoke<number>("set_watched", { accountId, repoIds });
}

/** Identifies a repo that was just unwatched so the backend can watch it again with its settings. */
export interface RestoreToken {
  account_id: string;
  repo_id: number;
}

/** Stops watching a repo and returns a restore token, or null when it was not watched. */
export function unwatchRepo(accountId: string, repoId: number): Promise<RestoreToken | null> {
  return invoke<RestoreToken | null>("unwatch_repo", { accountId, repoId });
}

/** Watches a repo again from the token `unwatchRepo` returned. Resolves false when it was not added. */
export function restoreWatchedRepo(token: RestoreToken): Promise<boolean> {
  return invoke<boolean>("restore_watched_repo", { token });
}

/** Sets the filter overrides of one organization; null fields inherit the global settings. */
export function setOrgFilters(host: string, owner: string, filters: OrgFilters): Promise<void> {
  return invoke("set_org_filters", { host, owner, filters });
}

/** Removes the per-repo branch, workflow and tag overrides so the repo inherits its organization. */
export function clearRepoOverrides(accountId: string, repoId: number): Promise<void> {
  return invoke("clear_repo_overrides", { accountId, repoId });
}

/** Sets the branch globs of one repo. Null inherits; an empty list watches the default branch only. */
export function setRepoBranches(accountId: string, repoId: number, patterns: string[] | null): Promise<void> {
  return invoke("set_repo_branches", { accountId, repoId, patterns });
}

export function updateSettings(settings: Settings): Promise<void> {
  return invoke("update_settings", { settings });
}

export function retrySecrets(): Promise<boolean> {
  return invoke<boolean>("retry_secrets");
}

/** Overwrites an unreadable Keychain item with an empty one. Every stored token is lost. */
export function resetSecrets(): Promise<boolean> {
  return invoke<boolean>("reset_secrets");
}

export interface RepoListProgress {
  account_id: string;
  loaded: number;
}

/** Fires after each page while an account's repository list loads. */
export function onRepoListProgress(handler: (progress: RepoListProgress) => void): Promise<UnlistenFn> {
  return listen<RepoListProgress>("repo-list-progress", (event) => handler(event.payload));
}

/** Looks for a new release now. Resolves null when Vigia is up to date. */
export function checkForUpdatesNow(): Promise<UpdateInfo | null> {
  return invoke<UpdateInfo | null>("check_for_updates_now");
}

/** Downloads and installs the available release, then relaunches. */
export function installUpdate(): Promise<void> {
  return invoke("install_update");
}

export interface UpdateProgress {
  downloaded: number;
  /** Null while the server has not reported a size. */
  total: number | null;
}

/** Fires while an update downloads. */
export function onUpdateProgress(handler: (progress: UpdateProgress) => void): Promise<UnlistenFn> {
  return listen<UpdateProgress>("update-progress", (event) => handler(event.payload));
}

export interface UpdateCheckResult {
  update: UpdateInfo | null;
  error: string | null;
}

/** Fires in the popup after the menu bar's "Check for Updates…" finishes. */
export function onUpdateCheckResult(handler: (result: UpdateCheckResult) => void): Promise<UnlistenFn> {
  return listen<UpdateCheckResult>("update-check-result", (event) => handler(event.payload));
}

/**
 * Turns a pending listener registration into a cleanup function. The listener is removed even
 * when cleanup runs before the registration resolves.
 */
export function subscribe(registration: Promise<UnlistenFn>): () => void {
  let cancelled = false;
  let unlisten: UnlistenFn | undefined;
  registration
    .then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    })
    .catch((e) => console.error("could not register event listener", e));
  return () => {
    cancelled = true;
    unlisten?.();
  };
}

/** Sets the ignored workflow globs of one repo. Null makes it inherit the organization or global list. */
export function setRepoIgnoredWorkflows(accountId: string, repoId: number, patterns: string[] | null): Promise<void> {
  return invoke("set_repo_ignored_workflows", { accountId, repoId, patterns });
}

/** Sets whether tag runs count for one repo. Null makes it inherit the organization or global setting. */
export function setRepoIncludeTags(accountId: string, repoId: number, include: boolean | null): Promise<void> {
  return invoke("set_repo_include_tags", { accountId, repoId, include });
}
