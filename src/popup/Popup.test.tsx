// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NO_ACCESS_NOTE, SAML_HINT } from "../lib/github";
import type { AccountSnapshot, Snapshot } from "../lib/snapshot";
import { makeAccount, makeRepo, makeRun, makeSnapshot } from "../test/fixtures";

const setSize = vi.fn(() => Promise.resolve());

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setSize }),
  LogicalSize: class {
    constructor(
      public width: number,
      public height: number,
    ) {}
  },
}));

vi.mock("../lib/tauri", () => ({
  getSnapshot: vi.fn(),
  onSnapshot: vi.fn(),
  subscribe: vi.fn(),
  refreshNow: vi.fn(),
  setPaused: vi.fn(),
  openSettings: vi.fn(),
  openUrl: vi.fn(),
  hidePopup: vi.fn(),
  restoreWatchedRepo: vi.fn(),
  onUpdateCheckResult: vi.fn(),
  onUpdateProgress: vi.fn(),
  installUpdate: vi.fn(),
}));

vi.mock("../lib/contextMenu", async () => {
  const actual = await vi.importActual<typeof import("../lib/contextMenu")>("../lib/contextMenu");
  return {
    ciPage: actual.ciPage,
    copyLink: vi.fn(),
    showRepoMenu: vi.fn(),
    stopWatching: vi.fn(),
  };
});

import { copyLink, showRepoMenu, stopWatching } from "../lib/contextMenu";
import {
  getSnapshot,
  hidePopup,
  installUpdate,
  onSnapshot,
  onUpdateCheckResult,
  onUpdateProgress,
  openSettings,
  openUrl,
  refreshNow,
  restoreWatchedRepo,
  setPaused,
  subscribe,
  type UpdateCheckResult,
  type UpdateProgress,
} from "../lib/tauri";
import { Popup } from "./Popup";

const NOW = new Date("2030-01-01T12:00:00Z");

function account(overrides: Partial<AccountSnapshot> = {}): AccountSnapshot {
  return makeAccount(overrides);
}

const run = makeRun;
const repo = makeRepo;

function snapshot(overrides: Partial<Snapshot> = {}): Snapshot {
  return makeSnapshot({
    color: "red",
    accounts: [account()],
    repos: [
      repo("acme/api", "failed", [
        run({ id: 1, state: "failed", name: "CI", branch: "main", group: "ci-main" }),
        run({ id: 2, state: "success", name: "Lint", branch: "dev", group: "lint-dev", url: "https://example.com/run/2" }),
      ]),
      repo("acme/web", "running", [run({ id: 3, state: "running", name: "Build", url: "https://example.com/run/3" })]),
      repo("acme/docs", "success", [run({ id: 4, state: "success", name: "Docs", url: "https://example.com/run/4" })]),
      repo("acme/idle", "none"),
    ],
    ...overrides,
  });
}

let pushSnapshot: (s: Snapshot) => void;
let pushCheckResult: (r: UpdateCheckResult) => void;
let pushProgress: (p: UpdateProgress) => void;
const unsubscribe = vi.fn();

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function mount(snap: Snapshot | null = snapshot()) {
  vi.mocked(getSnapshot).mockResolvedValue(snap as Snapshot);
  const view = render(<Popup />);
  await flush();
  return view;
}

function key(target: Element | Window | Document, k: string, init: KeyboardEventInit = {}) {
  fireEvent.keyDown(target, { key: k, ...init });
}

function sectionHeader(title: string): HTMLElement {
  return screen.getByRole("button", { name: new RegExp(`^${title}\\s*\\d+$`) });
}

const repoRows = () => Array.from(document.querySelectorAll<HTMLElement>('[data-nav="row"]'));
const runRows = () => Array.from(document.querySelectorAll<HTMLElement>('[data-nav="run"]'));
const firstRepoRow = () => repoRows()[0];
const mainPanel = () => screen.getByRole("main");

function expandPassing() {
  fireEvent.click(sectionHeader("Passing"));
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  vi.spyOn(console, "error").mockImplementation(() => undefined);
  Element.prototype.scrollIntoView = vi.fn();
  class FakeResizeObserver {
    static last: FakeResizeObserver | null = null;
    callback: () => void;
    observe = vi.fn();
    disconnect = vi.fn();
    constructor(cb: () => void) {
      this.callback = cb;
      FakeResizeObserver.last = this;
    }
  }
  vi.stubGlobal("ResizeObserver", FakeResizeObserver);
  vi.mocked(onSnapshot).mockImplementation(((handler: (s: Snapshot) => void) => {
    pushSnapshot = handler;
    return Promise.resolve(() => undefined);
  }) as unknown as typeof onSnapshot);
  vi.mocked(onUpdateCheckResult).mockImplementation(((handler: (r: UpdateCheckResult) => void) => {
    pushCheckResult = handler;
    return Promise.resolve(() => undefined);
  }) as unknown as typeof onUpdateCheckResult);
  vi.mocked(onUpdateProgress).mockImplementation(((handler: (p: UpdateProgress) => void) => {
    pushProgress = handler;
    return Promise.resolve(() => undefined);
  }) as unknown as typeof onUpdateProgress);
  vi.mocked(installUpdate).mockResolvedValue(undefined);
  vi.mocked(subscribe).mockImplementation(() => unsubscribe);
  vi.mocked(refreshNow).mockResolvedValue(undefined);
  vi.mocked(setPaused).mockResolvedValue(undefined);
  vi.mocked(openSettings).mockResolvedValue(undefined);
  vi.mocked(openUrl).mockResolvedValue(undefined);
  vi.mocked(hidePopup).mockResolvedValue(undefined);
  vi.mocked(restoreWatchedRepo).mockResolvedValue(true);
  vi.mocked(copyLink).mockResolvedValue(undefined);
  vi.mocked(showRepoMenu).mockResolvedValue(undefined);
  vi.mocked(stopWatching).mockResolvedValue(undefined);
  setSize.mockClear();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  vi.useRealTimers();
  delete (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__;
});

describe("Popup content", () => {
  it("renders an empty panel until the first snapshot arrives", () => {
    vi.mocked(getSnapshot).mockReturnValue(new Promise(() => undefined));
    const { container } = render(<Popup />);
    expect(container.querySelector("main")?.children.length).toBe(0);
  });

  it("keeps the page from scrolling while the popup is mounted", async () => {
    const { unmount } = await mount();
    expect(document.documentElement.classList.contains("popup-page")).toBe(true);
    unmount();
    expect(document.documentElement.classList.contains("popup-page")).toBe(false);
  });

  it("keeps all visually hidden text inside the list, which positions it", async () => {
    await mount();
    expandPassing();
    fireEvent.click(screen.getAllByRole("button", { name: /^Show runs for / })[0]);
    const list = mainPanel().querySelector('[data-fit="list"]');
    const hidden = Array.from(mainPanel().querySelectorAll(".visually-hidden"));
    expect(hidden.length).toBeGreaterThan(0);
    expect(hidden.every((el) => list?.contains(el))).toBe(true);
  });

  it("shows the add-account empty state and opens the add sheet", async () => {
    await mount(snapshot({ accounts: [], repos: [] }));
    expect(screen.getByRole("heading", { name: "Watch CI from the menu bar" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Add Account…" }));
    expect(openSettings).toHaveBeenCalledWith({ pane: "accounts", sheet: "add" });
    await flush();
    expect(hidePopup).toHaveBeenCalled();
  });

  it("shows the header summary, open sections and a collapsed Passing section", async () => {
    await mount();
    expect(screen.getByText("1 failed")).toBeTruthy();
    expect(sectionHeader("Failed").getAttribute("aria-expanded")).toBe("true");
    expect(sectionHeader("Passing").getAttribute("aria-expanded")).toBe("false");
    expect(screen.getByText("api")).toBeTruthy();
    expect(screen.queryByText("docs")).toBeNull();
    expect(screen.getByText("Updated 2m ago")).toBeTruthy();
  });

  it("opens and closes sections on click", async () => {
    await mount();
    expandPassing();
    expect(screen.getByText("docs")).toBeTruthy();
    expandPassing();
    expect(screen.queryByText("docs")).toBeNull();
  });

  it("opens and closes a section with the right and left arrow keys", async () => {
    await mount();
    const header = sectionHeader("Passing");
    key(header, "ArrowRight");
    expect(header.getAttribute("aria-expanded")).toBe("true");
    key(header, "ArrowRight");
    expect(header.getAttribute("aria-expanded")).toBe("true");
    key(header, "ArrowLeft");
    expect(header.getAttribute("aria-expanded")).toBe("false");
    key(header, "ArrowLeft");
    expect(header.getAttribute("aria-expanded")).toBe("false");
  });

  it("shows the account label in group titles when several accounts exist", async () => {
    const snap = snapshot({
      accounts: [account(), account({ id: "acc-2", label: "Example", kind: "gitlab" })],
      repos: [repo("acme/api", "failed", [run()]), repo("example/tool", "failed", [run()], {}, "acc-2")],
    });
    await mount(snap);
    expect(screen.getByText("Acme · acme")).toBeTruthy();
    expect(screen.getByText("Example · example")).toBeTruthy();
  });

  it("shows the no-repositories state with a link to the repos pane", async () => {
    await mount(snapshot({ repos: [], color: "gray" }));
    expect(screen.getByText("No repositories")).toBeTruthy();
    expect(screen.getByText("No repositories watched.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Choose Repositories…" }));
    expect(openSettings).toHaveBeenCalledWith({ pane: "repos" });
  });

  it("filters repositories and force-opens collapsed sections", async () => {
    await mount();
    const input = screen.getByLabelText("Filter repositories");
    fireEvent.change(input, { target: { value: "docs" } });
    expect(screen.getByText("docs")).toBeTruthy();
    expect(screen.queryByText("api")).toBeNull();
  });

  it("explains when the filter matches nothing", async () => {
    await mount();
    fireEvent.change(screen.getByLabelText("Filter repositories"), { target: { value: "zzz" } });
    expect(screen.getByText("No repositories match “zzz”.")).toBeTruthy();
  });

  it("shows paused state and the slowed-down interval in the footer", async () => {
    await mount(snapshot({ paused: true, accounts: [account({ effective_interval_secs: 300 })], generated_at: "garbage" }));
    const footer = screen.getByRole("contentinfo");
    expect(footer.textContent).toContain("Paused · ");
    expect(footer.textContent).toContain("Not updated yet");
    expect(footer.textContent).toContain("slowed to every 5 min");
    expect(footer.getAttribute("aria-live")).toBe("polite");
    expect(screen.getByRole("button", { name: "Resume" })).toBeTruthy();
  });

  it("replaces the snapshot when a new one is pushed", async () => {
    await mount();
    act(() => pushSnapshot(snapshot({ repos: [repo("acme/fresh", "failed", [run()])] })));
    expect(screen.getByText("fresh")).toBeTruthy();
    expect(screen.queryByText("api")).toBeNull();
  });

  it("refreshes the clock every 30 seconds and unsubscribes on unmount", async () => {
    const { unmount } = await mount();
    expect(screen.getByText("Updated 2m ago")).toBeTruthy();
    await act(async () => {
      vi.advanceTimersByTime(30_000 * 4);
    });
    expect(screen.getByText("Updated 4m ago")).toBeTruthy();
    unmount();
    expect(unsubscribe).toHaveBeenCalled();
  });

  it("stops the clock while the window is hidden and catches up when it returns", async () => {
    const hasFocus = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    await mount();
    hasFocus.mockReturnValue(false);
    act(() => {
      window.dispatchEvent(new Event("blur"));
    });
    await act(async () => {
      vi.advanceTimersByTime(30_000 * 4);
    });
    expect(screen.getByText("Updated 2m ago")).toBeTruthy();
    hasFocus.mockReturnValue(true);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(screen.getByText("Updated 4m ago")).toBeTruthy();
  });

  it("keeps unchanged rows when a snapshot changes one repository", async () => {
    await mount();
    const before = repoRows();
    const next = snapshot();
    next.repos[1] = repo("acme/web", "failed", [run({ id: 3, state: "failed", name: "Build" })]);
    act(() => pushSnapshot(next));
    const after = repoRows();
    expect(after[0]).toBe(before[0]);
    expect(after.filter((row, i) => row !== before[i] && before[i] !== undefined)).not.toHaveLength(0);
  });

  it("logs a failed initial snapshot request", async () => {
    vi.mocked(getSnapshot).mockRejectedValue(new Error("boom"));
    render(<Popup />);
    await flush();
    expect(console.error).toHaveBeenCalled();
  });
});

describe("Popup banners", () => {
  it("shows an auth banner for a rejected token", async () => {
    await mount(snapshot({ accounts: [account({ auth_error: true })] }));
    expect(screen.getByRole("alert").textContent).toContain("The token for “Acme” was rejected.");
  });

  it("shows a missing-token banner that opens the replace-token sheet", async () => {
    await mount(snapshot({ accounts: [account({ auth_error: true, auth_reason: "missing_token" })] }));
    expect(screen.getByRole("alert").textContent).toContain("No token is saved for “Acme”.");
    fireEvent.click(screen.getByRole("button", { name: "Replace Token…" }));
    expect(openSettings).toHaveBeenCalledWith({ pane: "accounts", sheet: "replace", accountId: "acc-1" });
  });

  it("shows a Keychain banner that opens Settings", async () => {
    await mount(snapshot({ accounts: [account({ auth_error: true, auth_reason: "keychain", keychain_denied: true })] }));
    expect(screen.getByRole("alert").textContent).toContain("Vigia can’t read the token for “Acme” from the Keychain.");
    fireEvent.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(openSettings).toHaveBeenCalledWith({ pane: "accounts" });
  });

  it("shows one banner when the Keychain is blocked", async () => {
    await mount(
      snapshot({ secrets_blocked: true, accounts: [account({ auth_error: true, auth_reason: "keychain", keychain_denied: true })] }),
    );
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("opens the replace-token sheet from the rejected-token banner", async () => {
    await mount(snapshot({ accounts: [account({ auth_error: true })] }));
    fireEvent.click(screen.getByRole("button", { name: "Replace Token…" }));
    expect(openSettings).toHaveBeenCalledWith({ pane: "accounts", sheet: "replace", accountId: "acc-1" });
  });

  it("shows unreachable, rate limited, config error and read-only banners", async () => {
    const until = Math.floor(NOW.getTime() / 1000) + 600;
    await mount(
      snapshot({
        accounts: [
          account({ id: "a", label: "Alpha", unreachable: true }),
          account({ id: "b", label: "Beta", kind: "gitlab", rate_limited_until: until }),
        ],
        config_error: "bad file",
        repos: [],
      }),
    );
    expect(screen.getByText("Alpha is unreachable. Showing the last known state.")).toBeTruthy();
    expect(screen.getByText(/Beta: paused until .* GitLab API limit\./)).toBeTruthy();
    const configBanner = screen.getByText(/couldn’t read its settings file/);
    expect(configBanner.getAttribute("title")).toBe("bad file");
    expect(configBanner.getAttribute("role")).toBe("status");
  });

  it("shows the read-only banner", async () => {
    await mount(snapshot({ config_read_only: true }));
    expect(screen.getByText(/saved by a newer version of Vigia/)).toBeTruthy();
  });

  it("opens the accounts pane from the keychain banner", async () => {
    await mount(snapshot({ secrets_blocked: true }));
    expect(screen.getByRole("alert").textContent).toContain("can’t read its tokens from the Keychain");
    fireEvent.click(screen.getByRole("button", { name: "Open Settings" }));
    expect(openSettings).toHaveBeenCalledWith({ pane: "accounts" });
  });
});

describe("Popup header actions", () => {
  it("refreshes, shows a busy state, and finishes when a snapshot arrives", async () => {
    await mount();
    const button = screen.getByRole("button", { name: "Refresh now" }) as HTMLButtonElement;
    fireEvent.click(button);
    expect(refreshNow).toHaveBeenCalled();
    expect(button.disabled).toBe(true);
    expect(button.getAttribute("aria-busy")).toBe("true");
        act(() => pushSnapshot(snapshot()));
    expect(button.disabled).toBe(false);
    expect(button.getAttribute("aria-busy")).toBe("false");
  });

  it("stops the busy state after the refresh timeout when no snapshot arrives", async () => {
    await mount();
    const button = screen.getByRole("button", { name: "Refresh now" }) as HTMLButtonElement;
    fireEvent.click(button);
    expect(button.disabled).toBe(true);
    act(() => {
      vi.advanceTimersByTime(10_000);
    });
    expect(button.disabled).toBe(false);
  });

  it("shows a friendly error when refresh fails and clears it later", async () => {
    vi.mocked(refreshNow).mockRejectedValue("rate limited");
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Refresh now" }));
    await flush();
    const alert = screen.getByRole("alert");
    expect(alert.textContent).toContain("The server is limiting requests");
    expect((screen.getByRole("button", { name: "Refresh now" }) as HTMLButtonElement).disabled).toBe(false);
    act(() => {
      vi.advanceTimersByTime(6000);
    });
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("disables refresh while paused", async () => {
    await mount(snapshot({ paused: true }));
    expect((screen.getByRole("button", { name: "Refresh now" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("pauses and resumes", async () => {
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(setPaused).toHaveBeenCalledWith(true);
  });

  it("resumes from the paused state", async () => {
    await mount(snapshot({ paused: true }));
    fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(setPaused).toHaveBeenCalledWith(false);
  });

  it("reports a pause failure", async () => {
    vi.mocked(setPaused).mockRejectedValue(new Error("nope"));
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    await flush();
    expect(screen.getByRole("alert").textContent).toContain("nope");
  });

  it("opens settings and hides the popup", async () => {
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(openSettings).toHaveBeenCalledWith(undefined);
    await flush();
    expect(hidePopup).toHaveBeenCalled();
  });

  it("reports a settings failure", async () => {
    vi.mocked(openSettings).mockRejectedValue(new Error("no window"));
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    await flush();
    expect(screen.getByRole("alert").textContent).toContain("no window");
  });

  it("marks the title as a polite live region", async () => {
    await mount();
    expect(screen.getByText("1 failed").parentElement?.getAttribute("aria-live")).toBe("polite");
  });
});

describe("Popup keyboard shortcuts", () => {
  it("refreshes on ⌘R and opens settings on ⌘,", async () => {
    await mount();
    key(window, "r", { metaKey: true });
    expect(refreshNow).toHaveBeenCalled();
    key(window, ",", { metaKey: true });
    expect(openSettings).toHaveBeenCalled();
  });

  it("focuses the filter on ⌘F", async () => {
    await mount();
    key(window, "f", { metaKey: true });
    expect(document.activeElement).toBe(screen.getByLabelText("Filter repositories"));
  });

  it("starts filtering when a character is typed outside the filter", async () => {
    await mount();
    key(window, "a");
    expect(document.activeElement).toBe(screen.getByLabelText("Filter repositories"));
  });

  it("ignores space, modified keys and keys typed inside the filter", async () => {
    await mount();
    const input = screen.getByLabelText("Filter repositories");
    key(window, " ");
    key(window, "x", { ctrlKey: true });
    key(window, "x", { altKey: true });
    key(window, "Shift");
    expect(document.activeElement).not.toBe(input);
    input.focus();
    key(input, "q");
    expect(document.activeElement).toBe(input);
  });

  it("clears the filter on Esc first, then closes the popup", async () => {
    await mount();
    const input = screen.getByLabelText("Filter repositories") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "api" } });
    key(window, "Escape");
    expect(input.value).toBe("");
    expect(hidePopup).not.toHaveBeenCalled();
    key(window, "Escape");
    expect(hidePopup).toHaveBeenCalledTimes(1);
  });

  it("reports a failure to hide the popup", async () => {
    vi.mocked(hidePopup).mockRejectedValue(new Error("cannot hide"));
    await mount();
    key(window, "Escape");
    await flush();
    expect(screen.getByRole("alert").textContent).toContain("cannot hide");
  });

  it("focuses the list on window focus and restarts the opening animation", async () => {
    await mount();
    const list = document.querySelector<HTMLElement>("[data-fit=list]") as HTMLElement;
    (document.activeElement as HTMLElement | null)?.blur();
    window.dispatchEvent(new Event("focus"));
    expect(document.activeElement).toBe(list);
    expect(mainPanel().classList.contains("popup--opening")).toBe(true);
  });

  it("keeps focus in the filter on window focus while filtering", async () => {
    await mount();
    const input = screen.getByLabelText("Filter repositories") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "a" } });
    input.focus();
    window.dispatchEvent(new Event("focus"));
    expect(document.activeElement).toBe(input);
  });
});

describe("Popup list navigation", () => {
  const rowNames = () => Array.from(document.querySelectorAll("[data-nav]")).map((n) => n.textContent);

  it("focuses the first repo row on the first down arrow, skipping section headers", async () => {
    await mount();
    const main = mainPanel();
    key(main, "ArrowDown");
    expect(document.activeElement).toBe(firstRepoRow());
    expect(document.activeElement?.textContent).toContain("api");
  });

  it("moves down and up through rows and back to the filter", async () => {
    await mount();
    const main = mainPanel();
    key(main, "ArrowDown");
    key(document.activeElement as HTMLElement, "ArrowDown");
    expect(document.activeElement?.textContent).toContain("Running");
    key(document.activeElement as HTMLElement, "ArrowUp");
    expect(document.activeElement?.textContent).toContain("api");
    key(document.activeElement as HTMLElement, "ArrowUp");
    expect(document.activeElement?.textContent).toContain("Failed");
    key(document.activeElement as HTMLElement, "ArrowUp");
    expect(document.activeElement).toBe(screen.getByLabelText("Filter repositories"));
  });

  it("stays on the last row at the bottom of the list", async () => {
    await mount();
    const rows = Array.from(document.querySelectorAll<HTMLElement>("[data-nav]"));
    const last = rows[rows.length - 1];
    last.focus();
    key(last, "ArrowDown");
    expect(document.activeElement).toBe(last);
  });

  it("ignores other keys and an empty list", async () => {
    await mount(snapshot({ repos: [] }));
    const main = mainPanel();
    key(main, "ArrowDown");
    key(main, "Tab");
    expect(repoRows()).toHaveLength(0);
  });

  it("falls back to the first nav element when no repo row exists", async () => {
    await mount(snapshot({ repos: [repo("acme/docs", "success", [run({ state: "success" })])] }));
    const main = mainPanel();
    key(main, "ArrowDown");
    expect(document.activeElement?.textContent).toContain("Passing");
    expect(rowNames()).toHaveLength(1);
  });

  it("expands with right arrow, shows runs, moves into them and back with left arrow", async () => {
    await mount();
    const apiRow = firstRepoRow();
    apiRow.focus();
    key(apiRow, "ArrowRight");
    const runs = runRows();
    expect(runs).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Show runs for api" }).getAttribute("aria-expanded")).toBe("true");
    key(apiRow, "ArrowDown");
    expect(document.activeElement).toBe(runs[0]);
    key(runs[0], "ArrowDown");
    expect(document.activeElement).toBe(runs[1]);
    key(runs[1], "ArrowLeft");
    expect(document.activeElement).toBe(apiRow);
    key(apiRow, "ArrowLeft");
    expect(runRows()).toHaveLength(0);
  });

  it("does not expand a repo with a single run", async () => {
    await mount();
    const web = repoRows()[1];
    key(web, "ArrowRight");
    expect(runRows()).toHaveLength(0);
    expect(screen.queryByRole("button", { name: "Show runs for web" })).toBeNull();
  });

  it("toggles runs with the disclosure button and labels runs for assistive tech", async () => {
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Show runs for api" }));
    expect(screen.getByRole("button", { name: "Open CI run in browser, Failed, main" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open Lint run in browser, Passing, dev" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Show runs for api" }));
    expect(runRows()).toHaveLength(0);
  });

  it("opens a repo URL on click and on Return", async () => {
    await mount();
    const row = firstRepoRow();
    fireEvent.click(row);
    expect(openUrl).toHaveBeenCalledWith("acc-1", "https://example.com/run/1");
  });

  it("opens a run URL on click", async () => {
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Show runs for api" }));
    fireEvent.click(runRows()[1]);
    expect(openUrl).toHaveBeenCalledWith("acc-1", "https://example.com/run/2");
  });

  it("opens the CI page of a repo with no runs", async () => {
    await mount(snapshot({ repos: [repo("acme/idle", "none")] }));
    fireEvent.click(sectionHeader("No runs"));
    fireEvent.click(firstRepoRow());
    expect(openUrl).toHaveBeenCalledWith("acc-1", "https://example.com/acme/idle/actions");
  });

  it("reports a failure to open a URL", async () => {
    vi.mocked(openUrl).mockRejectedValue(new Error("blocked"));
    await mount();
    fireEvent.click(firstRepoRow());
    await flush();
    expect(screen.getByRole("alert").textContent).toContain("blocked");
  });
});

describe("Popup row details", () => {
  it("shows stale repos with their last checked time", async () => {
    await mount(
      snapshot({
        repos: [repo("acme/api", "failed", [run()], { stale: true, last_checked: "2030-01-01T11:00:00Z" })],
      }),
    );
    const row = firstRepoRow();
    expect(within(row).getByText("Stale")).toBeTruthy();
    expect(within(row).getByText("1h ago")).toBeTruthy();
  });

  it("shows the note instead of the run summary", async () => {
    await mount(snapshot({ repos: [repo("acme/api", "error", [], { note: "Something odd" })] }));
    expect(screen.getByText("Something odd")).toBeTruthy();
  });

  it("shows the SAML hint for a GitHub repo without access, once expanded", async () => {
    await mount(snapshot({ repos: [repo("acme/api", "error", [], { note: NO_ACCESS_NOTE })] }));
    expect(screen.queryByText(SAML_HINT)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show runs for api" }));
    expect(screen.getByText(SAML_HINT)).toBeTruthy();
  });

  it("omits the SAML hint on GitLab accounts", async () => {
    await mount(
      snapshot({
        accounts: [account({ kind: "gitlab" })],
        repos: [repo("acme/api", "error", [], { note: NO_ACCESS_NOTE })],
      }),
    );
    expect(screen.queryByRole("button", { name: "Show runs for api" })).toBeNull();
  });

  it("treats a repo from an unknown account as GitHub", async () => {
    await mount(snapshot({ repos: [repo("acme/api", "failed", [run()], {}, "ghost")] }));
    expect(screen.getByText("api")).toBeTruthy();
  });

  it("pulses the dot of running repos and runs", async () => {
    await mount(
      snapshot({
        repos: [
          repo("acme/web", "running", [
            run({ id: 5, state: "running", group: "a" }),
            run({ id: 6, state: "queued", group: "b" }),
            run({ id: 7, state: "canceled", group: "c" }),
          ]),
        ],
      }),
    );
    expect(firstRepoRow().querySelector(".dot--pulse")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Show runs for web" }));
    expect(runRows().filter((r) => r.querySelector(".dot--pulse"))).toHaveLength(2);
  });

  it("colours error repos orange and runs-less repos gray", async () => {
    await mount(snapshot({ repos: [repo("acme/bad", "error", [], { note: "x" }), repo("acme/idle", "none")] }));
    expect(firstRepoRow().querySelector(".dot--orange")).toBeTruthy();
    fireEvent.click(sectionHeader("No runs"));
    expect(repoRows()[1].querySelector(".dot--gray")).toBeTruthy();
  });

  it("labels tag runs with their ref in the run list", async () => {
    await mount(
      snapshot({
        repos: [
          repo("acme/api", "failed", [
            run({ id: 1, branch: "v1.2.0", tag: true, group: "t1" }),
            run({ id: 2, branch: "main", group: "m", state: "success" }),
          ]),
        ],
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Show runs for api" }));
    expect(screen.getByText("v1.2.0")).toBeTruthy();
  });
});

describe("Popup context actions", () => {
  it("opens the repo menu on right click and on the Menu key or Shift+F10", async () => {
    await mount();
    const row = firstRepoRow();
    fireEvent.contextMenu(row);
    expect(showRepoMenu).toHaveBeenCalledTimes(1);
    key(row, "ContextMenu");
    expect(showRepoMenu).toHaveBeenCalledTimes(2);
    key(row, "F10", { shiftKey: true });
    expect(showRepoMenu).toHaveBeenCalledTimes(3);
    const options = vi.mocked(showRepoMenu).mock.calls[2][3] as { at?: { x: number; y: number } };
    expect(options.at).toBeTruthy();
  });

  it("opens the run menu on a run row", async () => {
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Show runs for api" }));
    const runRow = runRows()[0];
    fireEvent.contextMenu(runRow);
    key(runRow, "ContextMenu");
    expect(showRepoMenu).toHaveBeenCalledTimes(2);
  });

  it("copies the link on ⌘C and ignores plain C", async () => {
    await mount();
    const row = firstRepoRow();
    key(row, "c");
    expect(copyLink).not.toHaveBeenCalled();
    key(row, "c", { metaKey: true });
    expect(copyLink).toHaveBeenCalledTimes(1);
  });

  it("reports menu and copy failures", async () => {
    vi.mocked(showRepoMenu).mockRejectedValue(new Error("menu failed"));
    vi.mocked(copyLink).mockRejectedValue(new Error("copy failed"));
    await mount();
    const row = firstRepoRow();
    fireEvent.contextMenu(row);
    await flush();
    expect(screen.getByRole("alert").textContent).toContain("menu failed");
    key(row, "c", { metaKey: true });
    await flush();
    expect(screen.getByRole("alert").textContent).toContain("copy failed");
  });

  it("stops watching on ⌘⌫ from a run row too", async () => {
    await mount();
    fireEvent.click(screen.getByRole("button", { name: "Show runs for api" }));
    key(runRows()[0], "Backspace", { metaKey: true });
    expect(stopWatching).toHaveBeenCalledTimes(1);
  });

  it("ignores unrelated keys on a row", async () => {
    await mount();
    const row = firstRepoRow();
    key(row, "Backspace");
    key(row, "Enter");
    expect(stopWatching).not.toHaveBeenCalled();
  });

  describe("Stop Watching", () => {
    const stoppedRepo = { token: { account_id: "acc-1", repo_id: 1 }, name: "api" };

    function stopOnKey() {
      vi.mocked(stopWatching).mockImplementation(async (_repo, onStopped) => {
        onStopped(stoppedRepo);
      });
      const row = firstRepoRow();
      key(row, "Backspace", { metaKey: true });
    }

    it("shows an undo banner that expires", async () => {
      await mount();
      stopOnKey();
      await flush();
      expect(screen.getByText("Stopped watching api.")).toBeTruthy();
      act(() => {
        vi.advanceTimersByTime(8000);
      });
      expect(screen.queryByText("Stopped watching api.")).toBeNull();
    });

    it("restores the repo on Undo", async () => {
      await mount();
      stopOnKey();
      await flush();
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
      await flush();
      expect(restoreWatchedRepo).toHaveBeenCalledWith(stoppedRepo.token);
      expect(screen.queryByText("Stopped watching api.")).toBeNull();
      expect(screen.queryByRole("alert")).toBeNull();
    });

    it("explains when the repo could not be restored", async () => {
      vi.mocked(restoreWatchedRepo).mockResolvedValue(false);
      await mount();
      stopOnKey();
      await flush();
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
      await flush();
      expect(screen.getByRole("alert").textContent).toBe("Couldn't restore it. Add it again in Settings.");
    });

    it("reports a restore that throws", async () => {
      vi.mocked(restoreWatchedRepo).mockRejectedValue(new Error("restore broke"));
      await mount();
      stopOnKey();
      await flush();
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
      await flush();
      expect(screen.getByRole("alert").textContent).toContain("restore broke");
    });
  });
});

describe("Popup window sizing", () => {
  function layout(inner: number, visible: number) {
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) {
      return this.dataset.fit === "inner" ? inner : 100;
    });
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(function (this: HTMLElement) {
      return this.dataset.fit === "list" ? visible : 0;
    });
    vi.spyOn(window, "getComputedStyle").mockReturnValue({ paddingTop: "4px", paddingBottom: "4px" } as CSSStyleDeclaration);
  }

  it("sizes the window to the content with the list's full height swapped in", async () => {
    layout(300, 208);
    window.innerHeight = 400;
    await mount();
    // 400 - (208 - 8) + 300 = 500
    expect(setSize).toHaveBeenLastCalledWith(expect.objectContaining({ width: 380, height: 500 }));
  });

  it("clamps the height to the maximum and the minimum", async () => {
    layout(2000, 208);
    window.innerHeight = 400;
    await mount();
    expect(setSize).toHaveBeenLastCalledWith(expect.objectContaining({ height: 520 }));
    cleanup();
    setSize.mockClear();
    layout(0, 208);
    await mount();
    expect(setSize).toHaveBeenLastCalledWith(expect.objectContaining({ height: 200 }));
  });

  it("only resizes when the height changes, and follows list resizes", async () => {
    layout(300, 208);
    window.innerHeight = 400;
    await mount();
    const calls = setSize.mock.calls.length;
    fireEvent.change(screen.getByLabelText("Filter repositories"), { target: { value: "" } });
    expect(setSize).toHaveBeenCalledTimes(calls);
    layout(350, 208);
    const Observer = ResizeObserver as unknown as { last: { callback: () => void } };
    act(() => Observer.last.callback());
    expect(setSize).toHaveBeenCalledTimes(calls + 1);
    expect(setSize).toHaveBeenLastCalledWith(expect.objectContaining({ height: 520 }));
  });

  it("sizes the empty state from its content", async () => {
    layout(0, 0);
    await mount(snapshot({ accounts: [], repos: [] }));
    // empty offsetHeight 100 + 64
    expect(setSize).toHaveBeenCalledWith(expect.objectContaining({ height: 200 }));
  });

  it("logs a resize failure", async () => {
    layout(300, 208);
    setSize.mockReturnValueOnce(Promise.reject(new Error("resize failed")));
    await mount();
    expect(console.error).toHaveBeenCalled();
  });

  it("skips resizing in preview mode", async () => {
    (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__ = true;
    layout(300, 208);
    await mount();
    expect(setSize).not.toHaveBeenCalled();
    const Observer = ResizeObserver as unknown as { last: unknown };
    expect(Observer.last).toBeNull();
  });
});

describe("Popup updates", () => {
  const update = { version: "2.0.0", notes: null };

  function installButton() {
    return screen.getByRole("button", { name: /^(Install and Relaunch|Downloading|Installing)/ });
  }

  it("shows no update banner without an update", async () => {
    await mount();
    expect(screen.queryByText(/is available/)).toBeNull();
  });

  it("announces an available update with an install button", async () => {
    await mount(snapshot({ update }));
    expect(within(screen.getByText("Vigia 2.0.0 is available.").closest("[role=status]") as HTMLElement).getByRole("button").textContent).toBe(
      "Install and Relaunch",
    );
  });

  it("follows the download and then installs", async () => {
    await mount(snapshot({ update }));
    fireEvent.click(installButton());
    expect(installUpdate).toHaveBeenCalledTimes(1);
    expect((installButton() as HTMLButtonElement).disabled).toBe(true);
    expect(installButton().textContent).toBe("Downloading…");
    act(() => pushProgress({ downloaded: 250, total: 1000 }));
    expect(installButton().textContent).toBe("Downloading… 25%");
    act(() => pushProgress({ downloaded: 100, total: null }));
    expect(installButton().textContent).toBe("Downloading…");
    act(() => pushProgress({ downloaded: 1000, total: 1000 }));
    expect(installButton().textContent).toBe("Installing…");
  });

  it("ignores progress while nothing is installing", async () => {
    await mount(snapshot({ update }));
    act(() => pushProgress({ downloaded: 500, total: 1000 }));
    expect(installButton().textContent).toBe("Install and Relaunch");
  });

  it("shows an install failure and enables the button again", async () => {
    vi.mocked(installUpdate).mockRejectedValue({ kind: "network", message: "offline" });
    await mount(snapshot({ update }));
    fireEvent.click(installButton());
    await flush();
    expect(screen.getByRole("alert")).toBeTruthy();
    expect((installButton() as HTMLButtonElement).disabled).toBe(false);
    expect(installButton().textContent).toBe("Install and Relaunch");
  });

  it("shows the banner for an update found from the menu bar", async () => {
    await mount();
    act(() => pushCheckResult({ update, error: null }));
    expect(screen.getByText("Vigia 2.0.0 is available.")).toBeTruthy();
  });

  it("shows a transient up to date message", async () => {
    await mount();
    act(() => pushCheckResult({ update: null, error: null }));
    expect(screen.getByText("Vigia is up to date.").closest("[role=status]")).toBeTruthy();
    act(() => {
      vi.advanceTimersByTime(6000);
    });
    expect(screen.queryByText("Vigia is up to date.")).toBeNull();
  });

  it("shows a failed check as an alert", async () => {
    await mount();
    act(() => pushCheckResult({ update: null, error: "offline" }));
    expect(screen.getByRole("alert").textContent).toContain("offline");
  });
});
