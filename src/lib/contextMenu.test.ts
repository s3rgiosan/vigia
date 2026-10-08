// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ciPage, copyLink, linkFor, showRepoMenu, stopWatching } from "./contextMenu";
import type { RepoSnapshot, Run } from "./snapshot";
import { openUrl, unwatchRepo, type RestoreToken } from "./tauri";

interface FakeItem {
  opts: { text?: string; item?: string; accelerator?: string; action?: () => void };
  close: ReturnType<typeof vi.fn>;
}

const mocks = vi.hoisted(() => ({
  popup: vi.fn(),
  menuClose: vi.fn(),
  menuItems: [] as unknown[],
  writeText: vi.fn(),
}));

vi.mock("@tauri-apps/api/menu", () => {
  const make = (opts: FakeItem["opts"]): FakeItem => ({ opts, close: vi.fn(() => Promise.resolve()) });
  return {
    MenuItem: { new: vi.fn((opts: FakeItem["opts"]) => Promise.resolve(make(opts))) },
    PredefinedMenuItem: { new: vi.fn((opts: FakeItem["opts"]) => Promise.resolve(make(opts))) },
    Menu: {
      new: vi.fn((opts: { items: unknown[] }) => {
        mocks.menuItems = opts.items;
        return Promise.resolve({ popup: mocks.popup, close: mocks.menuClose });
      }),
    },
  };
});
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ writeText: mocks.writeText }));
vi.mock("./tauri", () => ({ openUrl: vi.fn(), unwatchRepo: vi.fn() }));

function repoAt(webUrl: string): RepoSnapshot {
  return {
    account_id: "a",
    repo: { id: 1, full_name: "acme/widgets", web_url: webUrl, default_branch: "main" },
    state: { status: "none", representative: null, groups: [], last_checked: null, stale: false, note: null },
  };
}

describe("ciPage", () => {
  it("points GitHub repos at Actions", () => {
    expect(ciPage(repoAt("https://github.com/acme/widgets"), true)).toBe("https://github.com/acme/widgets/actions");
  });

  it("ignores trailing slashes", () => {
    expect(ciPage(repoAt("https://github.com/acme/widgets//"), true)).toBe("https://github.com/acme/widgets/actions");
  });

  it("points GitLab repos at Pipelines", () => {
    expect(ciPage(repoAt("https://gitlab.example.com/group/sub/widgets/"), false)).toBe(
      "https://gitlab.example.com/group/sub/widgets/-/pipelines",
    );
  });
});

describe("linkFor", () => {
  const run: Run = {
    id: 1,
    attempt: 1,
    state: "failed",
    branch: "main",
    group: "ci",
    name: "CI",
    url: "https://github.com/acme/widgets/actions/runs/1",
    updated_at: "2026-06-01T11:00:00Z",
    pull_request: false,
    fork: false,
    tag: false,
  };

  it("prefers the run link", () => {
    expect(linkFor(repoAt("https://github.com/acme/widgets"), run)).toBe(run.url);
  });

  it("falls back to the repository link", () => {
    expect(linkFor(repoAt("https://github.com/acme/widgets"), null)).toBe("https://github.com/acme/widgets");
  });
});

const RUN: Run = {
  id: 7,
  attempt: 1,
  state: "failed",
  branch: "main",
  group: "ci",
  name: "CI",
  url: "https://github.com/acme/widgets/actions/runs/7",
  updated_at: "2026-06-01T11:00:00Z",
  pull_request: false,
  fork: false,
  tag: false,
};
const TOKEN: RestoreToken = { account_id: "a", repo_id: 1 };

function items(): FakeItem[] {
  return mocks.menuItems as FakeItem[];
}

function labels(): string[] {
  return items().map((i) => i.opts.text ?? i.opts.item ?? "");
}

function find(text: string): FakeItem {
  const found = items().find((i) => i.opts.text === text);
  if (!found) {
    throw new Error(`no menu item ${text}`);
  }
  return found;
}

describe("showRepoMenu", () => {
  const repo = repoAt("https://github.com/acme/widgets/");

  beforeEach(() => {
    vi.useFakeTimers();
    vi.mocked(openUrl).mockResolvedValue(undefined);
    mocks.writeText.mockResolvedValue(undefined);
    mocks.popup.mockResolvedValue(undefined);
    mocks.menuClose.mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.clearAllMocks();
    delete (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__;
  });

  it("builds the run menu for a GitHub repo", async () => {
    await showRepoMenu(repo, RUN, true, { onError: vi.fn(), onStopped: vi.fn() });
    expect(labels()).toEqual([
      "Open Run",
      "Open Actions",
      "Open Repository",
      "Separator",
      "Copy Run Link",
      "Separator",
      "Stop Watching",
    ]);
  });

  it("builds the repository menu for a GitLab repo without a run", async () => {
    await showRepoMenu(repo, null, false, { onError: vi.fn(), onStopped: vi.fn() });
    expect(labels()).toEqual([
      "Open Pipelines",
      "Open Repository",
      "Separator",
      "Copy Repository Link",
      "Separator",
      "Stop Watching",
    ]);
  });

  it("opens the run, CI page and repository through the account", async () => {
    await showRepoMenu(repo, RUN, true, { onError: vi.fn(), onStopped: vi.fn() });
    find("Open Run").opts.action?.();
    find("Open Actions").opts.action?.();
    find("Open Repository").opts.action?.();
    expect(openUrl).toHaveBeenNthCalledWith(1, "a", RUN.url);
    expect(openUrl).toHaveBeenNthCalledWith(2, "a", "https://github.com/acme/widgets/actions");
    expect(openUrl).toHaveBeenNthCalledWith(3, "a", "https://github.com/acme/widgets/");
  });

  it("copies the run link, or the repository link when there is no run", async () => {
    await showRepoMenu(repo, RUN, true, { onError: vi.fn(), onStopped: vi.fn() });
    find("Copy Run Link").opts.action?.();
    expect(mocks.writeText).toHaveBeenCalledWith(RUN.url);
    await showRepoMenu(repo, null, true, { onError: vi.fn(), onStopped: vi.fn() });
    find("Copy Repository Link").opts.action?.();
    expect(mocks.writeText).toHaveBeenLastCalledWith("https://github.com/acme/widgets/");
  });

  it("reports the restore token after stopping", async () => {
    vi.mocked(unwatchRepo).mockResolvedValue(TOKEN);
    const onStopped = vi.fn();
    await showRepoMenu(repo, RUN, true, { onError: vi.fn(), onStopped });
    find("Stop Watching").opts.action?.();
    await vi.advanceTimersByTimeAsync(0);
    expect(unwatchRepo).toHaveBeenCalledWith("a", 1);
    expect(onStopped).toHaveBeenCalledWith({ token: TOKEN, name: "widgets" });
  });

  it("routes failed actions to onError", async () => {
    const boom = new Error("boom");
    vi.mocked(openUrl).mockRejectedValue(boom);
    const onError = vi.fn();
    await showRepoMenu(repo, RUN, true, { onError, onStopped: vi.fn() });
    find("Open Repository").opts.action?.();
    await vi.advanceTimersByTimeAsync(0);
    expect(onError).toHaveBeenCalledWith(boom);
  });

  it("pops up at the given position", async () => {
    await showRepoMenu(repo, RUN, true, { onError: vi.fn(), onStopped: vi.fn(), at: { x: 10, y: 20 } });
    const position = mocks.popup.mock.calls[0][0];
    expect(position.x).toBe(10);
    expect(position.y).toBe(20);
  });

  it("pops up at the pointer without a position", async () => {
    await showRepoMenu(repo, RUN, true, { onError: vi.fn(), onStopped: vi.fn() });
    expect(mocks.popup).toHaveBeenCalledWith(undefined);
  });

  it("closes the menu and, a tick later, its items", async () => {
    await showRepoMenu(repo, RUN, true, { onError: vi.fn(), onStopped: vi.fn() });
    expect(mocks.menuClose).toHaveBeenCalledTimes(1);
    expect(items().every((i) => i.close.mock.calls.length === 0)).toBe(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(items().every((i) => i.close.mock.calls.length === 1)).toBe(true);
  });

  it("closes the menu even when popup fails", async () => {
    mocks.popup.mockRejectedValue(new Error("no window"));
    await expect(showRepoMenu(repo, RUN, true, { onError: vi.fn(), onStopped: vi.fn() })).rejects.toThrow("no window");
    expect(mocks.menuClose).toHaveBeenCalledTimes(1);
  });

  it("reports items that fail to close", async () => {
    const onError = vi.fn();
    await showRepoMenu(repo, RUN, true, { onError, onStopped: vi.fn() });
    const failure = new Error("close failed");
    items()[0].close.mockRejectedValue(failure);
    await vi.advanceTimersByTimeAsync(0);
    expect(onError).toHaveBeenCalledWith(failure);
  });

  it("only logs in the browser preview", async () => {
    (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__ = true;
    const info = vi.spyOn(console, "info").mockImplementation(() => undefined);
    await showRepoMenu(repo, RUN, true, { onError: vi.fn(), onStopped: vi.fn() });
    expect(info).toHaveBeenCalledWith("[preview] context menu for", "acme/widgets");
    expect(mocks.popup).not.toHaveBeenCalled();
    info.mockRestore();
  });
});

describe("stopWatching", () => {
  afterEach(() => vi.clearAllMocks());

  it("passes the restore token and repository name on", async () => {
    vi.mocked(unwatchRepo).mockResolvedValue(TOKEN);
    const onStopped = vi.fn();
    await stopWatching(repoAt("https://github.com/acme/widgets"), onStopped);
    expect(onStopped).toHaveBeenCalledWith({ token: TOKEN, name: "widgets" });
  });

  it("stays silent when the repo was not watched", async () => {
    vi.mocked(unwatchRepo).mockResolvedValue(null);
    const onStopped = vi.fn();
    await stopWatching(repoAt("https://github.com/acme/widgets"), onStopped);
    expect(onStopped).not.toHaveBeenCalled();
  });
});

describe("copyLink", () => {
  it("writes the run link to the clipboard", async () => {
    mocks.writeText.mockResolvedValue(undefined);
    await copyLink(repoAt("https://github.com/acme/widgets"), RUN);
    expect(mocks.writeText).toHaveBeenCalledWith(RUN.url);
  });
});
