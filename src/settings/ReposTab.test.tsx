// @vitest-environment jsdom
import { act, cleanup, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PickerRepo, RepoListProgress, SettingsView, WatchedRepo } from "../lib/tauri";
import { makeSettingsAccount, makeView as makeBaseView, makeWatchedRepo } from "../test/fixtures";
import { makeSettingsContext, renderWithSettings } from "../test/settings";

const getSettings = vi.fn();
const listPickerRepos = vi.fn();
const setWatched = vi.fn();
let progressHandler: ((progress: RepoListProgress) => void) | null = null;

vi.mock("../lib/tauri", () => ({
  getSettings: () => getSettings(),
  listPickerRepos: (...args: unknown[]) => listPickerRepos(...args),
  onRepoListProgress: (handler: (progress: RepoListProgress) => void) => {
    progressHandler = handler;
    return Promise.resolve(() => undefined);
  },
  setWatched: (...args: unknown[]) => setWatched(...args),
  subscribe: () => () => undefined,
}));

import { ReposTab } from "./ReposTab";

let context = makeSettingsContext();

function pickerRepos(names: string[], overrides: Record<string, string> = {}): PickerRepo[] {
  return names.map((full, i) => ({
    repo: { id: i + 1, full_name: full, web_url: "", default_branch: "main" },
    watched: false,
    watched_via: overrides[full] ?? null,
  }));
}

const defaultRepos = pickerRepos(["acme/alpha", "acme/beta", "acme/gamma"]);

const github = makeSettingsAccount({ id: "a1" });
const gitlab = makeSettingsAccount({ id: "a2", kind: "gitlab", label: "Example", base_url: "https://git.example.com" });

/** A watched repo entry of an account, by picker id. */
function entry(id: number, accountId = "a1"): WatchedRepo {
  const base = makeWatchedRepo(`acme/repo-${id}`, { account_id: accountId });
  return { ...base, repo: { ...base.repo, id } };
}

function makeView(patch: Partial<SettingsView> = {}): SettingsView {
  return makeBaseView({ accounts: [github], ...patch });
}

async function settle(ms = 0) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

async function renderTab(view = makeView()) {
  const result = renderWithSettings(<ReposTab />, { view });
  context = result.context;
  await settle();
  return { ...result, update: (v: SettingsView) => result.update({ view: v }) };
}

function list() {
  return screen.getByRole("listbox", { name: "Repositories" });
}

function options() {
  return within(list()).getAllByRole("option");
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  vi.useFakeTimers();
  for (const fn of [getSettings, listPickerRepos, setWatched]) {
    fn.mockReset();
  }
  getSettings.mockResolvedValue(makeView());
  listPickerRepos.mockResolvedValue(defaultRepos);
  setWatched.mockResolvedValue(0);
  progressHandler = null;
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("repository picker", () => {
  it("is one multiselectable listbox with positioned options", async () => {
    await renderTab();
    expect(list().getAttribute("aria-multiselectable")).toBe("true");
    const items = options();
    expect(items).toHaveLength(4);
    expect(items[1].getAttribute("aria-posinset")).toBe("2");
    expect(items[1].getAttribute("aria-setsize")).toBe("4");
    expect(document.querySelectorAll('input[type="checkbox"][tabindex="-1"]')).toHaveLength(4);
  });

  it("moves the active option with arrows and toggles with Space", async () => {
    await renderTab();
    fireEvent.keyDown(list(), { key: "ArrowDown" });
    fireEvent.keyDown(list(), { key: "ArrowDown" });
    expect(list().getAttribute("aria-activedescendant")).toBe("picker-opt-2");
    fireEvent.keyDown(list(), { key: " " });
    expect(options()[2].getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(list(), { key: "End" });
    expect(list().getAttribute("aria-activedescendant")).toBe("picker-opt-3");
    fireEvent.keyDown(list(), { key: "Home" });
    expect(list().getAttribute("aria-activedescendant")).toBe("picker-opt-0");
    fireEvent.keyDown(list(), { key: "ArrowUp" });
    expect(list().getAttribute("aria-activedescendant")).toBe("picker-opt-0");
  });

  it("selects every repository of an organisation from its header", async () => {
    await renderTab();
    fireEvent.keyDown(list(), { key: " " });
    for (const option of options()) {
      expect(option.getAttribute("aria-selected")).toBe("true");
    }
  });

  it("pages through a long list and scrolls the active row into view", async () => {
    const names = Array.from({ length: 80 }, (_, i) => `acme/repo-${String(i).padStart(2, "0")}`);
    listPickerRepos.mockResolvedValue(pickerRepos(names));
    await renderTab();
    fireEvent.keyDown(list(), { key: "PageDown" });
    expect(list().getAttribute("aria-activedescendant")).toBe("picker-opt-1");
    fireEvent.keyDown(list(), { key: "End" });
    const scroller = list().parentElement as HTMLElement;
    expect(scroller.scrollTop).toBeGreaterThan(0);
    fireEvent.keyDown(list(), { key: "PageUp" });
    fireEvent.keyDown(list(), { key: "Home" });
    expect(scroller.scrollTop).toBe(4);
    fireEvent.scroll(scroller);
    expect(list().getAttribute("aria-activedescendant")).toBe("picker-opt-0");
  });

  it("leaves unrelated keys to the browser", async () => {
    await renderTab();
    const notPrevented = fireEvent.keyDown(list(), { key: "a" });
    expect(notPrevented).toBe(true);
  });

  it("measures the list with a resize observer when available", async () => {
    const observe = vi.fn();
    const disconnect = vi.fn();
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe = observe;
        disconnect = disconnect;
      },
    );
    const { unmount } = await renderTab();
    expect(observe).toHaveBeenCalled();
    unmount();
    expect(disconnect).toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});

describe("loading", () => {
  it("shows progress for the current account only", async () => {
    const pending = deferred<PickerRepo[]>();
    listPickerRepos.mockReturnValue(pending.promise);
    await renderTab();
    expect(screen.getAllByText(/Loading repositories…/).length).toBeGreaterThan(0);
    act(() => progressHandler?.({ account_id: "other", loaded: 99 }));
    expect(screen.queryByText(/99 loaded/)).toBeNull();
    act(() => progressHandler?.({ account_id: "a1", loaded: 40 }));
    expect(screen.getAllByText(/Loading repositories… 40 loaded/).length).toBeGreaterThan(0);
    await act(async () => pending.resolve(defaultRepos));
    expect(screen.queryByText(/Loading repositories/)).toBeNull();
    expect(screen.getByText("0 watched")).toBeTruthy();
  });

  it("reports a failed load and keeps showing the loading state", async () => {
    listPickerRepos.mockRejectedValue("network error: down");
    await renderTab();
    expect(context.setError).toHaveBeenCalledWith("Can't reach the server. Check the address and your connection.");
  });

  it("reloads the list on request and disables the button meanwhile", async () => {
    await renderTab();
    const pending = deferred<PickerRepo[]>();
    listPickerRepos.mockReturnValue(pending.promise);
    const reload = screen.getByLabelText("Reload repositories") as HTMLButtonElement;
    fireEvent.click(reload);
    await settle();
    expect(listPickerRepos).toHaveBeenLastCalledWith("a1", true);
    expect(reload.disabled).toBe(true);
    expect(screen.getAllByText(/Loading repositories/).length).toBeGreaterThan(0);
    await act(async () => pending.resolve(pickerRepos(["acme/delta"])));
    expect(reload.disabled).toBe(false);
    expect(options()).toHaveLength(2);
  });

  it("keeps the current list when a reload fails", async () => {
    await renderTab();
    listPickerRepos.mockRejectedValue("rate limited");
    fireEvent.click(screen.getByLabelText("Reload repositories"));
    await settle();
    expect(context.setError).toHaveBeenLastCalledWith("The server is limiting requests. Vigia will try again shortly.");
    expect(options()).toHaveLength(4);
  });

  it("shows when an account has no repositories", async () => {
    listPickerRepos.mockResolvedValue([]);
    await renderTab();
    expect(screen.getByText("This account has no repositories.")).toBeTruthy();
    expect(list().getAttribute("tabindex")).toBe("-1");
  });
});

describe("search", () => {
  it("filters the list and resets the cursor", async () => {
    await renderTab();
    fireEvent.keyDown(list(), { key: "End" });
    fireEvent.change(screen.getByLabelText("Search repositories"), { target: { value: "bet" } });
    expect(options()).toHaveLength(2);
    expect(list().getAttribute("aria-activedescendant")).toBe("picker-opt-0");
  });

  it("explains an empty result", async () => {
    await renderTab();
    fireEvent.change(screen.getByLabelText("Search repositories"), { target: { value: "zzz" } });
    expect(screen.getByText("No repositories match “zzz”.")).toBeTruthy();
  });
});

describe("selecting and saving", () => {
  it("saves a pick after a pause and reloads the settings", async () => {
    await renderTab();
    fireEvent.click(options()[1]);
    expect(screen.getByText("Saving…")).toBeTruthy();
    expect(screen.getByText("1 watched")).toBeTruthy();
    await settle(699);
    expect(setWatched).not.toHaveBeenCalled();
    await settle(2);
    expect(setWatched).toHaveBeenCalledWith("a1", [1]);
    expect(screen.getByText("Saved")).toBeTruthy();
    expect(context.setError).toHaveBeenCalledWith(null);
    expect(context.reload).toHaveBeenCalled();
  });

  it("batches a burst of clicks into one save and toggles a pick off again", async () => {
    await renderTab();
    fireEvent.click(options()[1]);
    await settle(300);
    fireEvent.click(options()[2]);
    fireEvent.click(options()[1]);
    await settle(1000);
    expect(setWatched).toHaveBeenCalledTimes(1);
    expect(setWatched).toHaveBeenCalledWith("a1", [2]);
  });

  it("adds picks on top of repos watched elsewhere since the list loaded", async () => {
    getSettings.mockResolvedValue(
      makeView({ repos: [entry(99), entry(5, "other")] }),
    );
    await renderTab();
    fireEvent.click(options()[1]);
    await settle(800);
    expect(setWatched).toHaveBeenCalledWith("a1", [99, 1]);
  });

  it("drops repos that were unticked even when the server still has them", async () => {
    listPickerRepos.mockResolvedValue(pickerRepos(["acme/alpha", "acme/beta"]).map((r) => ({ ...r, watched: true })));
    const both = makeView({ repos: [entry(1), entry(2)] });
    getSettings.mockResolvedValue(both);
    await renderTab(both);
    fireEvent.click(options()[1]);
    await settle(800);
    expect(setWatched).toHaveBeenCalledWith("a1", [2]);
  });

  it("keeps edits made while a save is in flight", async () => {
    const pending = deferred<number>();
    setWatched.mockReturnValueOnce(pending.promise);
    await renderTab();
    fireEvent.click(options()[1]);
    await settle(800);
    fireEvent.click(options()[2]);
    fireEvent.click(options()[1]);
    await act(async () => pending.resolve(1));
    expect(screen.getByText("1 watched")).toBeTruthy();
    expect(options()[2].getAttribute("aria-selected")).toBe("true");
    expect(options()[1].getAttribute("aria-selected")).toBe("false");
    expect(screen.getByText("Saving…")).toBeTruthy();
    await settle(800);
    expect(setWatched).toHaveBeenCalledTimes(2);
  });

  it("shows a failure, reports it and retries the pick when the pane closes", async () => {
    setWatched.mockRejectedValueOnce("server error: boom");
    const { unmount } = await renderTab();
    fireEvent.click(options()[1]);
    await settle(800);
    expect(screen.getByText("Not saved")).toBeTruthy();
    expect(context.setError).toHaveBeenCalledWith("The server had a problem. Try again in a moment.");
    expect(context.reload).not.toHaveBeenCalled();
    unmount();
    await settle();
    expect(setWatched).toHaveBeenCalledTimes(2);
  });

  it("saves a pending pick when the pane closes", async () => {
    const { unmount } = await renderTab();
    fireEvent.click(options()[1]);
    unmount();
    await settle();
    expect(setWatched).toHaveBeenCalledWith("a1", [1]);
  });

  it("selects and clears a whole organisation from its header", async () => {
    await renderTab();
    fireEvent.click(options()[0]);
    expect(options()[0].getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("3 watched")).toBeTruthy();
    fireEvent.click(options()[0]);
    expect(screen.getByText("0 watched")).toBeTruthy();
  });

  it("labels an organisation with some repositories selected", async () => {
    await renderTab();
    fireEvent.click(options()[2]);
    expect(options()[0].getAttribute("aria-label")).toBe("acme, 3 repositories, some selected");
    expect((options()[0].querySelector("input") as HTMLInputElement).indeterminate).toBe(true);
  });

  it("separates organisations and lists them alphabetically", async () => {
    listPickerRepos.mockResolvedValue(pickerRepos(["zeta/one", "acme/one"]));
    await renderTab();
    const headers = options().filter((o) => o.className.includes("picker-row--org"));
    expect(headers.map((h) => h.textContent)).toEqual(["acme1", "zeta1"]);
    expect(headers[0].className).not.toContain("picker-row--sep");
    expect(headers[1].className).toContain("picker-row--sep");
  });

  it("follows changes made elsewhere while nothing is pending", async () => {
    const { update } = await renderTab();
    expect(screen.getByText("0 watched")).toBeTruthy();
    update(makeView({ repos: [entry(2)] }));
    expect(screen.getByText("1 watched")).toBeTruthy();
    expect(options()[2].getAttribute("aria-selected")).toBe("true");
  });

  it("shows a pending pick on top of changes made elsewhere", async () => {
    const { update } = await renderTab();
    fireEvent.click(options()[1]);
    update(makeView({ repos: [entry(3)] }));
    expect(options()[1].getAttribute("aria-selected")).toBe("true");
    expect(options()[3].getAttribute("aria-selected")).toBe("true");
  });

  it("keeps an unticked repo off while the reload after a save still lists it", async () => {
    const one = makeView({ repos: [entry(1)] });
    const { update } = await renderTab(one);
    const pending = deferred<number>();
    setWatched.mockReturnValueOnce(pending.promise);
    fireEvent.click(options()[2]);
    await settle(800);
    fireEvent.click(options()[1]);
    update(makeView({ repos: [entry(1), entry(2)] }));
    await act(async () => pending.resolve(2));
    expect(options()[1].getAttribute("aria-selected")).toBe("false");
    getSettings.mockResolvedValue(makeView({ repos: [entry(1), entry(2)] }));
    await settle(800);
    expect(setWatched).toHaveBeenLastCalledWith("a1", [2]);
  });
});

describe("repositories watched through another account", () => {
  beforeEach(() => {
    listPickerRepos.mockResolvedValue(pickerRepos(["acme/alpha", "acme/beta"], { "acme/beta": "Other" }));
  });

  it("shows where the repo is watched and ignores clicks and Space on it", async () => {
    await renderTab();
    const row = options()[2];
    expect(row.textContent).toContain("Watched via Other");
    expect(row.getAttribute("aria-disabled")).toBe("true");
    fireEvent.click(row);
    expect(screen.getByText("0 watched")).toBeTruthy();
    fireEvent.keyDown(list(), { key: "End" });
    fireEvent.keyDown(list(), { key: " " });
    expect(screen.getByText("0 watched")).toBeTruthy();
  });

  it("selects only the available repositories from the organisation header", async () => {
    await renderTab();
    fireEvent.click(options()[0]);
    expect(screen.getByText("1 watched")).toBeTruthy();
    expect(options()[0].getAttribute("aria-selected")).toBe("true");
  });
});

describe("read-only config", () => {
  it("does not change picks", async () => {
    await renderTab(makeView({ read_only: true }));
    fireEvent.click(options()[1]);
    fireEvent.click(options()[0]);
    fireEvent.keyDown(list(), { key: " " });
    expect(screen.getByText("0 watched")).toBeTruthy();
    expect(options()[0].getAttribute("aria-disabled")).toBe("true");
    expect(options()[1].getAttribute("aria-disabled")).toBe("true");
  });
});

describe("accounts", () => {
  const twoAccounts = () => makeView({ accounts: [github, gitlab] });

  it("shows the SAML hint for GitHub accounts only", async () => {
    await renderTab(twoAccounts());
    expect(document.querySelector(".repos__hint")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Account"), { target: { value: "a2" } });
    await settle();
    expect(document.querySelector(".repos__hint")).toBeNull();
  });

  it("saves a pending pick for the old account before loading the new one", async () => {
    await renderTab(twoAccounts());
    fireEvent.click(options()[1]);
    listPickerRepos.mockResolvedValue(pickerRepos(["example/solo"]));
    fireEvent.change(screen.getByLabelText("Account"), { target: { value: "a2" } });
    await settle();
    expect(setWatched).toHaveBeenCalledWith("a1", [1]);
    expect(listPickerRepos).toHaveBeenLastCalledWith("a2", false);
    expect(options()).toHaveLength(2);
    expect(screen.getByText("0 watched")).toBeTruthy();
    expect(screen.queryByText("Saved")).toBeNull();
  });

  it("drops a failed save of the previous account instead of retrying it on the new one", async () => {
    setWatched.mockRejectedValue("server error: boom");
    await renderTab(twoAccounts());
    fireEvent.click(options()[1]);
    fireEvent.change(screen.getByLabelText("Account"), { target: { value: "a2" } });
    await settle();
    expect(setWatched).toHaveBeenCalledTimes(1);
    expect(screen.queryByText("Not saved")).toBeNull();
  });

  it("ignores a list that arrives after the account changed", async () => {
    const slow = deferred<PickerRepo[]>();
    listPickerRepos.mockReturnValueOnce(slow.promise).mockResolvedValue(pickerRepos(["example/solo"]));
    await renderTab(twoAccounts());
    fireEvent.change(screen.getByLabelText("Account"), { target: { value: "a2" } });
    await settle();
    await act(async () => slow.resolve(defaultRepos));
    expect(options()).toHaveLength(2);
    expect(options()[1].textContent).toContain("solo");
  });

  it("falls back to the first account when the chosen one disappears", async () => {
    const { update } = await renderTab(twoAccounts());
    fireEvent.change(screen.getByLabelText("Account"), { target: { value: "a2" } });
    await settle();
    update(makeView());
    await settle();
    expect((screen.getByLabelText("Account") as HTMLSelectElement).value).toBe("a1");
    expect(listPickerRepos).toHaveBeenLastCalledWith("a1", false);
  });

  it("ignores a failure from a load that was replaced", async () => {
    const slow = deferred<PickerRepo[]>();
    listPickerRepos.mockReturnValueOnce(slow.promise).mockResolvedValue(pickerRepos(["example/solo"]));
    await renderTab(twoAccounts());
    fireEvent.change(screen.getByLabelText("Account"), { target: { value: "a2" } });
    await settle();
    await act(async () => slow.reject("network error"));
    expect(context.setError).not.toHaveBeenCalledWith(expect.stringContaining("Can't reach"));
  });
});

describe("no accounts", () => {
  it("offers Add Account", async () => {
    await renderTab(makeView({ accounts: [] }));
    fireEvent.click(screen.getByRole("button", { name: "Add Account…" }));
    expect(context.requestAdd).toHaveBeenCalled();
    expect(listPickerRepos).not.toHaveBeenCalled();
  });

  it("disables Add Account when locked or the Keychain is blocked", async () => {
    const { update } = await renderTab(makeView({ accounts: [], read_only: true }));
    expect((screen.getByRole("button", { name: "Add Account…" }) as HTMLButtonElement).disabled).toBe(true);
    update(makeView({ accounts: [], secrets_blocked: true }));
    expect((screen.getByRole("button", { name: "Add Account…" }) as HTMLButtonElement).disabled).toBe(true);
  });
});
