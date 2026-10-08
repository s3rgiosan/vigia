import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import * as api from "./tauri";
import type { NewAccount, RestoreToken, Settings } from "./tauri";
import { makeSnapshot, makeView } from "../test/fixtures";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

const NEW_ACCOUNT: NewAccount = { kind: "github", label: "Acme", token: "github_pat_x", allow_insecure: false };
const TOKEN: RestoreToken = { account_id: "a", repo_id: 1 };
const SETTINGS = { poll_interval_secs: 60 } as Settings;

beforeEach(() => {
  mocks.invoke.mockResolvedValue("result");
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("command wrappers", () => {
  const calls: [string, () => Promise<unknown>, string, unknown?][] = [
    ["refreshNow", () => api.refreshNow(), "refresh_now"],
    ["setPaused", () => api.setPaused(true), "set_paused", { paused: true }],
    ["openUrl", () => api.openUrl("a", "https://example.com"), "open_url", { accountId: "a", url: "https://example.com" }],
    ["hidePopup", () => api.hidePopup(), "hide_popup"],
    ["openGithubTokenPage", () => api.openGithubTokenPage("acme"), "open_github_token_page", { owner: "acme" }],
    ["selectSettingsPane", () => api.selectSettingsPane("repos"), "select_settings_pane", { pane: "repos" }],
    ["setSettingsToolbarEnabled", () => api.setSettingsToolbarEnabled(true), "set_settings_toolbar_enabled", { enabled: true }],
    ["openSettings with target", () => api.openSettings({ pane: "accounts", sheet: "replace", accountId: "a" }), "open_settings", { pane: "accounts", sheet: "replace", accountId: "a" }],
    ["openSettings without target", () => api.openSettings(), "open_settings", { pane: null, sheet: null, accountId: null }],
    ["openSettings with empty target", () => api.openSettings({}), "open_settings", { pane: null, sheet: null, accountId: null }],
    ["testConnection", () => api.testConnection(NEW_ACCOUNT), "test_connection", { input: NEW_ACCOUNT }],
    ["addAccount", () => api.addAccount(NEW_ACCOUNT), "add_account", { input: NEW_ACCOUNT }],
    ["renameAccount", () => api.renameAccount("a", "Acme"), "rename_account", { accountId: "a", label: "Acme" }],
    ["replaceToken", () => api.replaceToken("a", "t"), "replace_token", { accountId: "a", token: "t" }],
    ["deleteAccount", () => api.deleteAccount("a"), "delete_account", { accountId: "a" }],
    ["deleteAllAccounts", () => api.deleteAllAccounts(), "delete_all_accounts"],
    ["listPickerRepos", () => api.listPickerRepos("a", true), "list_picker_repos", { accountId: "a", refresh: true }],
    ["setWatched", () => api.setWatched("a", [1, 2]), "set_watched", { accountId: "a", repoIds: [1, 2] }],
    ["unwatchRepo", () => api.unwatchRepo("a", 1), "unwatch_repo", { accountId: "a", repoId: 1 }],
    ["restoreWatchedRepo", () => api.restoreWatchedRepo(TOKEN), "restore_watched_repo", { token: TOKEN }],
    ["clearRepoOverrides", () => api.clearRepoOverrides("a", 1), "clear_repo_overrides", { accountId: "a", repoId: 1 }],
    ["setRepoBranches", () => api.setRepoBranches("a", 1, ["main"]), "set_repo_branches", { accountId: "a", repoId: 1, patterns: ["main"] }],
    [
      "setOrgFilters",
      () => api.setOrgFilters("github.com", "acme", { branch_patterns: null, ignored_workflows: [], include_tags: true }),
      "set_org_filters",
      { host: "github.com", owner: "acme", filters: { branch_patterns: null, ignored_workflows: [], include_tags: true } },
    ],
    ["updateSettings", () => api.updateSettings(SETTINGS), "update_settings", { settings: SETTINGS }],
    ["checkForUpdatesNow", () => api.checkForUpdatesNow(), "check_for_updates_now"],
    ["installUpdate", () => api.installUpdate(), "install_update"],
    ["retrySecrets", () => api.retrySecrets(), "retry_secrets"],
    ["resetSecrets", () => api.resetSecrets(), "reset_secrets"],
    ["setRepoIgnoredWorkflows", () => api.setRepoIgnoredWorkflows("a", 1, null), "set_repo_ignored_workflows", { accountId: "a", repoId: 1, patterns: null }],
    ["setRepoIncludeTags", () => api.setRepoIncludeTags("a", 1, false), "set_repo_include_tags", { accountId: "a", repoId: 1, include: false }],
  ];

  it.each(calls)("%s invokes the matching command", async (_name, call, command, args) => {
    const result = await call();
    expect(result).toBe("result");
    if (args === undefined) {
      expect(mocks.invoke).toHaveBeenCalledWith(command);
    } else {
      expect(mocks.invoke).toHaveBeenCalledWith(command, args);
    }
  });
});

describe("validated payloads", () => {
  it("getSnapshot resolves a valid snapshot", async () => {
    const snapshot = makeSnapshot();
    mocks.invoke.mockResolvedValue(snapshot);
    await expect(api.getSnapshot()).resolves.toBe(snapshot);
    expect(mocks.invoke).toHaveBeenCalledWith("get_snapshot");
  });

  it("getSnapshot rejects an invalid snapshot with an internal error", async () => {
    mocks.invoke.mockResolvedValue({ nope: true });
    await expect(api.getSnapshot()).rejects.toMatchObject({ kind: "internal" });
  });

  it("getSettings resolves a valid view", async () => {
    const view = makeView();
    mocks.invoke.mockResolvedValue(view);
    await expect(api.getSettings()).resolves.toBe(view);
    expect(mocks.invoke).toHaveBeenCalledWith("get_settings");
  });

  it("getSettings rejects an invalid view with an internal error", async () => {
    mocks.invoke.mockResolvedValue(null);
    await expect(api.getSettings()).rejects.toMatchObject({ kind: "internal" });
  });
});

describe("event listeners", () => {
  it("onSnapshot hands a valid payload to the handler", async () => {
    const snapshot = makeSnapshot();
    mocks.listen.mockImplementation((_event: string, cb: (e: { payload: unknown }) => void) => {
      cb({ payload: snapshot });
      return Promise.resolve(() => undefined);
    });
    const handler = vi.fn();
    await api.onSnapshot(handler);
    expect(mocks.listen.mock.calls[0][0]).toBe("snapshot-updated");
    expect(handler).toHaveBeenCalledWith(snapshot);
  });

  it("onSnapshot logs and drops a malformed payload", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    mocks.listen.mockImplementation((_event: string, cb: (e: { payload: unknown }) => void) => {
      cb({ payload: { marker: 1 } });
      return Promise.resolve(() => undefined);
    });
    const handler = vi.fn();
    await api.onSnapshot(handler);
    expect(handler).not.toHaveBeenCalled();
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });

  it.each([
    ["onUpdateProgress", "update-progress", { downloaded: 1, total: null }, api.onUpdateProgress],
    ["onUpdateCheckResult", "update-check-result", { update: null, error: null }, api.onUpdateCheckResult],
  ] as const)("%s hands the payload to the handler", async (_name, event, payload, wrapper) => {
    mocks.listen.mockImplementation((_event: string, cb: (e: { payload: unknown }) => void) => {
      cb({ payload });
      return Promise.resolve(() => undefined);
    });
    const handler = vi.fn();
    await (wrapper as (h: typeof handler) => Promise<unknown>)(handler);
    expect(mocks.listen.mock.calls[0][0]).toBe(event);
    expect(handler).toHaveBeenCalledWith(payload);
  });

  it("onSettingsPane hands the pane to the handler", async () => {
    mocks.listen.mockImplementation((_event: string, cb: (e: { payload: unknown }) => void) => {
      cb({ payload: "repos" });
      return Promise.resolve(() => undefined);
    });
    const handler = vi.fn();
    await api.onSettingsPane(handler);
    expect(mocks.listen.mock.calls[0][0]).toBe(api.SETTINGS_PANE_EVENT);
    expect(handler).toHaveBeenCalledWith("repos");
  });

  it("onSettingsOpen hands the payload to the handler", async () => {
    mocks.listen.mockImplementation((_event: string, cb: (e: { payload: unknown }) => void) => {
      cb({ payload: { pane: "repos", sheet: null, account_id: null } });
      return Promise.resolve(() => undefined);
    });
    const handler = vi.fn();
    await api.onSettingsOpen(handler);
    expect(mocks.listen.mock.calls[0][0]).toBe("settings-open");
    expect(handler).toHaveBeenCalledWith({ pane: "repos", sheet: null, account_id: null });
  });

  it("onRepoListProgress hands the payload to the handler", async () => {
    mocks.listen.mockImplementation((_event: string, cb: (e: { payload: unknown }) => void) => {
      cb({ payload: { account_id: "a", loaded: 30 } });
      return Promise.resolve(() => undefined);
    });
    const handler = vi.fn();
    await api.onRepoListProgress(handler);
    expect(mocks.listen.mock.calls[0][0]).toBe("repo-list-progress");
    expect(handler).toHaveBeenCalledWith({ account_id: "a", loaded: 30 });
  });
});

describe("subscribe", () => {
  it("removes the listener on cleanup after registration", async () => {
    const unlisten = vi.fn();
    const cleanup = api.subscribe(Promise.resolve(unlisten));
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten).not.toHaveBeenCalled();
    cleanup();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("removes the listener when cleanup runs before registration resolves", async () => {
    const unlisten = vi.fn();
    let resolve: (fn: () => void) => void = () => undefined;
    const pending = new Promise<() => void>((r) => {
      resolve = r;
    });
    const cleanup = api.subscribe(pending);
    cleanup();
    resolve(unlisten);
    await pending;
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("logs a failed registration and still returns a safe cleanup", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const failure = new Error("denied");
    const cleanup = api.subscribe(Promise.reject(failure));
    await new Promise((r) => setTimeout(r, 0));
    expect(error).toHaveBeenCalledWith("could not register event listener", failure);
    expect(() => cleanup()).not.toThrow();
    error.mockRestore();
  });
});
