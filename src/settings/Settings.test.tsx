// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Snapshot } from "../lib/snapshot";
import type { SettingsOpen, SettingsView } from "../lib/tauri";
import { makeSnapshot, makeView } from "../test/fixtures";
import type { SheetRequest } from "./SettingsContext";

const getSettings = vi.fn();
const getSnapshot = vi.fn();
const retrySecrets = vi.fn();
const resetSecrets = vi.fn();
const selectSettingsPane = vi.fn();
const setSettingsToolbarEnabled = vi.fn();
const setTitle = vi.fn();
let openHandler: ((payload: SettingsOpen) => void) | null = null;
let snapshotHandler: ((snapshot: Snapshot) => void) | null = null;
let paneHandler: ((pane: string) => void) | null = null;

vi.mock("../lib/tauri", () => ({
  getSettings: () => getSettings(),
  getSnapshot: () => getSnapshot(),
  retrySecrets: () => retrySecrets(),
  resetSecrets: () => resetSecrets(),
  selectSettingsPane: (pane: string) => selectSettingsPane(pane),
  setSettingsToolbarEnabled: (enabled: boolean) => setSettingsToolbarEnabled(enabled),
  onSettingsOpen: (handler: (payload: SettingsOpen) => void) => {
    openHandler = handler;
    return Promise.resolve(() => undefined);
  },
  onSettingsPane: (handler: (pane: string) => void) => {
    paneHandler = handler;
    return Promise.resolve(() => undefined);
  },
  onSnapshot: (handler: (snapshot: Snapshot) => void) => {
    snapshotHandler = handler;
    return Promise.resolve(() => undefined);
  },
  subscribe: () => () => undefined,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTitle: (title: string) => setTitle(title) }),
}));

// The build platform, switchable per test; read on every render.
let platform = "darwin";
vi.mock("../lib/platform", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/platform")>();
  return {
    ...actual,
    get IS_MACOS() {
      return platform === "darwin";
    },
    get SECRET_STORE() {
      return actual.secretStoreNames(platform);
    },
  };
});

vi.mock("./useSettingsSaver", () => ({ useSettingsSaver: () => vi.fn(async () => undefined) }));

function stub(name: string) {
  return function Stub() {
    const { reload, setError, requestAdd, readOnly } = useSettings();
    return (
      <div data-testid={`pane-${name}`}>
        <span>{`${name} readOnly=${readOnly}`}</span>
        <button type="button" onClick={() => void reload()}>{`${name} reload`}</button>
        <button type="button" onClick={() => setError("boom")}>{`${name} fail`}</button>
        <button type="button" onClick={requestAdd}>{`${name} add`}</button>
      </div>
    );
  };
}

vi.mock("./AccountsTab", () => ({
  AccountsTab: ({ request, onRequestHandled }: { request: SheetRequest | null; onRequestHandled: () => void }) => {
    const Pane = stub("accounts");
    return (
      <>
        <Pane />
        <span>{`request=${request ? `${request.sheet}:${request.accountId}` : "none"}`}</span>
        <button type="button" onClick={onRequestHandled}>
          handle request
        </button>
      </>
    );
  },
}));
vi.mock("./ReposTab", () => ({ ReposTab: stub("repos") }));
vi.mock("./BranchesTab", () => ({ BranchesTab: stub("branches") }));
vi.mock("./GeneralTab", () => ({ GeneralTab: stub("general") }));

import { useSettings } from "./SettingsContext";

type SettingsComponent = typeof import("./Settings").Settings;

function target(patch: Partial<SettingsOpen>): SettingsOpen {
  return { pane: null, sheet: null, account_id: null, ...patch };
}

async function load(options: { preview?: boolean; search?: string } = {}): Promise<SettingsComponent> {
  const w = window as unknown as Record<string, unknown>;
  if (options.preview) {
    w.__VIGIA_PREVIEW__ = true;
  } else {
    delete w.__VIGIA_PREVIEW__;
  }
  window.history.replaceState(null, "", options.search ?? "/");
  return (await import("./Settings")).Settings;
}

async function mount(options: { preview?: boolean; search?: string } = {}) {
  const Settings = await load(options);
  render(<Settings />);
  await act(async () => undefined);
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
  localStorage.clear();
  getSettings.mockReset().mockResolvedValue(makeView());
  getSnapshot.mockReset().mockResolvedValue(makeSnapshot());
  retrySecrets.mockReset().mockResolvedValue(true);
  resetSecrets.mockReset().mockResolvedValue(true);
  selectSettingsPane.mockReset().mockResolvedValue(undefined);
  setSettingsToolbarEnabled.mockReset().mockResolvedValue(undefined);
  setTitle.mockReset().mockResolvedValue(undefined);
  openHandler = null;
  snapshotHandler = null;
  paneHandler = null;
});

afterEach(() => {
  cleanup();
  delete (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__;
});

describe("pane selection", () => {
  it("starts on Accounts and names the pane in the window title", async () => {
    await mount();
    expect(screen.getByTestId("pane-accounts")).toBeTruthy();
    expect(document.title).toBe("Accounts");
    expect(setTitle).toHaveBeenCalledWith("Accounts");
    expect(selectSettingsPane).toHaveBeenCalledWith("accounts");
  });

  it("opens the pane named in the URL", async () => {
    await mount({ search: "/?pane=general" });
    expect(screen.getByTestId("pane-general")).toBeTruthy();
    expect(localStorage.getItem("vigia.settings.tab")).toBe("general");
  });

  it("falls back to the remembered pane, then to Accounts for unknown names", async () => {
    localStorage.setItem("vigia.settings.tab", "branches");
    await mount();
    expect(screen.getByTestId("pane-branches")).toBeTruthy();
    cleanup();
    localStorage.setItem("vigia.settings.tab", "nonsense");
    await mount({ search: "/?pane=nope" });
    expect(screen.getByTestId("pane-accounts")).toBeTruthy();
  });

  it("asks the Accounts pane for the sheet the URL names", async () => {
    localStorage.setItem("vigia.settings.tab", "general");
    await mount({ search: "/?sheet=add" });
    expect(screen.getByText("request=add:null")).toBeTruthy();
    fireEvent.click(screen.getByText("handle request"));
    expect(screen.getByText("request=none")).toBeTruthy();
    cleanup();
    await mount({ search: "/?view=settings&pane=accounts&sheet=replace&account=acc%3A2" });
    expect(screen.getByText("request=replace:acc:2")).toBeTruthy();
  });

  it("ignores a Replace Token link without an account and unknown sheets", async () => {
    await mount({ search: "/?pane=general&sheet=replace&account_id=acc-1" });
    expect(screen.getByTestId("pane-general")).toBeTruthy();
    cleanup();
    await mount({ search: "/?sheet=other" });
    expect(screen.queryByText(/request=(add|replace)/)).toBeNull();
  });

  it("switches panes with Command plus a digit and ignores other keys", async () => {
    await mount();
    fireEvent.keyDown(window, { key: "3", metaKey: true });
    expect(screen.getByTestId("pane-branches")).toBeTruthy();
    fireEvent.keyDown(window, { key: "2" });
    expect(screen.getByTestId("pane-branches")).toBeTruthy();
    fireEvent.keyDown(window, { key: "9", metaKey: true });
    fireEvent.keyDown(window, { key: "0", metaKey: true });
    expect(screen.getByTestId("pane-branches")).toBeTruthy();
    fireEvent.keyDown(window, { key: "4", metaKey: true });
    expect(screen.getByTestId("pane-general")).toBeTruthy();
    fireEvent.keyDown(window, { key: "2", metaKey: true });
    expect(screen.getByTestId("pane-repos")).toBeTruthy();
    expect(setTitle).toHaveBeenLastCalledWith("Repositories");
  });

  it("follows clicks on the native toolbar", async () => {
    await mount();
    act(() => paneHandler?.("repos"));
    expect(screen.getByTestId("pane-repos")).toBeTruthy();
    act(() => paneHandler?.("unknown"));
    expect(screen.getByTestId("pane-repos")).toBeTruthy();
  });

  it("shows a pane or the add sheet when the settings-open event arrives", async () => {
    await mount();
    act(() => openHandler?.(target({ pane: "general" })));
    expect(screen.getByTestId("pane-general")).toBeTruthy();
    act(() => openHandler?.(target({})));
    expect(screen.getByTestId("pane-general")).toBeTruthy();
    act(() => openHandler?.(target({ pane: "repos", sheet: "add" })));
    expect(screen.getByText("request=add:null")).toBeTruthy();
    fireEvent.click(screen.getByText("handle request"));
    fireEvent.keyDown(window, { key: "4", metaKey: true });
    act(() => openHandler?.(target({ sheet: "replace", account_id: "acc-1" })));
    expect(screen.getByText("request=replace:acc-1")).toBeTruthy();
  });
});

describe("open sheets", () => {
  async function openResetSheet() {
    getSettings.mockResolvedValue(makeView({ secrets_blocked: true }));
    await mount();
    fireEvent.click(screen.getByText("Reset Keychain Item…"));
    expect(screen.getByRole("alertdialog", { name: "Reset Keychain Item?" })).toBeTruthy();
  }

  it("ignores pane switches and re-highlights the current pane", async () => {
    await openResetSheet();
    selectSettingsPane.mockClear();
    fireEvent.keyDown(window, { key: "2", metaKey: true });
    expect(screen.getByTestId("pane-accounts")).toBeTruthy();
    expect(selectSettingsPane).toHaveBeenCalledWith("accounts");
    act(() => paneHandler?.("general"));
    expect(screen.getByTestId("pane-accounts")).toBeTruthy();
  });

  it("ignores settings-open requests", async () => {
    await openResetSheet();
    act(() => openHandler?.(target({ pane: "general" })));
    expect(screen.getByTestId("pane-accounts")).toBeTruthy();
  });
});

describe("Linux and Windows", () => {
  afterEach(() => {
    platform = "darwin";
  });

  it("draws its own pane switcher below the title bar", async () => {
    platform = "windows";
    await mount();
    const nav = screen.getByRole("navigation", { name: "Settings panes" });
    expect(nav.className).toBe("toolbar toolbar--titled");
    fireEvent.click(screen.getByRole("button", { name: /General/ }));
    expect(screen.getByTestId("pane-general")).toBeTruthy();
    expect(selectSettingsPane).toHaveBeenLastCalledWith("general");
  });

  it("names the token store as the OS does", async () => {
    platform = "linux";
    getSettings.mockResolvedValue(makeView({ secrets_blocked: true }));
    await mount();
    expect(screen.getByText("Vigia can’t read its tokens from the keyring.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Reset Keyring Item…" }));
    expect(screen.getByRole("alertdialog", { name: "Reset Keyring Item?" })).toBeTruthy();
  });
});

describe("browser preview", () => {
  it("draws its own pane switcher and does not call the native window", async () => {
    await mount({ preview: true });
    expect(setTitle).not.toHaveBeenCalled();
    expect(selectSettingsPane).not.toHaveBeenCalled();
    const nav = screen.getByRole("navigation", { name: "Settings panes" });
    expect(nav.querySelector("[aria-current=page]")?.textContent).toBe("Accounts");
    fireEvent.click(screen.getByRole("button", { name: /General/ }));
    expect(screen.getByTestId("pane-general")).toBeTruthy();
    expect(document.title).toBe("General");
  });

  it("does not subscribe to native events", async () => {
    await mount({ preview: true });
    expect(paneHandler).toBeNull();
    expect(openHandler).toBeNull();
  });

  it("ignores pane switches while a sheet is open without calling the native window", async () => {
    getSettings.mockResolvedValue(makeView({ secrets_blocked: true }));
    await mount({ preview: true });
    fireEvent.click(screen.getByText("Reset Keychain Item…"));
    fireEvent.click(screen.getByRole("button", { name: /Filters/ }));
    expect(screen.getByTestId("pane-accounts")).toBeTruthy();
    expect(selectSettingsPane).not.toHaveBeenCalled();
  });
});

describe("banners", () => {
  it("shows the read-only notice", async () => {
    getSettings.mockResolvedValue(makeView({ read_only: true }));
    await mount();
    expect(screen.getByText(/saved by a newer version of Vigia/)).toBeTruthy();
  });

  it("shows the config error instead of the read-only notice", async () => {
    getSettings.mockResolvedValue(makeView({ read_only: true }));
    getSnapshot.mockResolvedValue(makeSnapshot({ config_error: "bad toml" }));
    await mount();
    const banner = screen.getByText("Vigia couldn’t read its settings file, so changes won’t be saved.");
    expect(banner.closest("[title]")?.getAttribute("title")).toBe("bad toml");
    expect(screen.queryByText(/saved by a newer version/)).toBeNull();
  });

  it("picks up snapshots pushed later", async () => {
    await mount();
    act(() => snapshotHandler?.(makeSnapshot({ config_error: "late" })));
    expect(screen.getByTitle("late")).toBeTruthy();
  });

  it("logs a snapshot that fails to load", async () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => undefined);
    getSnapshot.mockRejectedValue(new Error("nope"));
    await mount();
    expect(log).toHaveBeenCalled();
    log.mockRestore();
  });

  it("shows a friendly error from a failed load and dismisses it", async () => {
    getSettings.mockRejectedValue("network error: down");
    await mount();
    expect(screen.getByRole("alert").textContent).toContain("Can't reach the server");
    fireEvent.click(screen.getByLabelText("Dismiss"));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows errors reported by a pane", async () => {
    await mount();
    fireEvent.click(screen.getByText("accounts fail"));
    expect(screen.getByRole("alert").textContent).toContain("boom");
  });

  it("switches to Accounts when a pane asks to add an account", async () => {
    await mount({ search: "/?pane=repos" });
    fireEvent.click(screen.getByText("repos add"));
    expect(screen.getByText("request=add:null")).toBeTruthy();
  });

  it("tells the panes when the config is read-only", async () => {
    getSettings.mockResolvedValue(makeView({ read_only: true }));
    await mount();
    expect(screen.getByText("accounts readOnly=true")).toBeTruthy();
  });
});

describe("stale responses", () => {
  it("ignores a settings response that a newer request has replaced", async () => {
    const slow = deferred<SettingsView>();
    getSettings.mockReset();
    getSettings
      .mockResolvedValueOnce(makeView())
      .mockReturnValueOnce(slow.promise)
      .mockResolvedValueOnce(makeView({ read_only: true }));
    await mount();
    fireEvent.click(screen.getByText("accounts reload"));
    fireEvent.click(screen.getByText("accounts reload"));
    await act(async () => undefined);
    await act(async () => {
      slow.resolve(makeView({ secrets_blocked: true }));
    });
    expect(screen.queryByText("Try Again")).toBeNull();
  });

  it("ignores a failure that a newer request has replaced", async () => {
    const slow = deferred<SettingsView>();
    getSettings.mockReset();
    getSettings
      .mockResolvedValueOnce(makeView())
      .mockReturnValueOnce(slow.promise)
      .mockResolvedValueOnce(makeView());
    await mount();
    fireEvent.click(screen.getByText("accounts reload"));
    fireEvent.click(screen.getByText("accounts reload"));
    await act(async () => undefined);
    await act(async () => {
      slow.reject("network error");
    });
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("Keychain banner", () => {
  beforeEach(() => {
    getSettings.mockResolvedValue(makeView({ secrets_blocked: true }));
  });

  it("retries and reloads the settings", async () => {
    await mount();
    getSettings.mockClear();
    fireEvent.click(screen.getByText("Try Again"));
    await waitFor(() => expect(getSettings).toHaveBeenCalledTimes(1));
    expect(retrySecrets).toHaveBeenCalled();
  });

  it("shows a friendly error when the retry fails", async () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => undefined);
    retrySecrets.mockRejectedValue("keychain access denied");
    await mount();
    fireEvent.click(screen.getByText("Try Again"));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Vigia can't use the Keychain"));
    log.mockRestore();
  });

  it("resets the item, closes the sheet and reloads", async () => {
    await mount();
    fireEvent.click(screen.getByText("Reset Keychain Item…"));
    getSettings.mockResolvedValue(makeView());
    fireEvent.click(screen.getByRole("button", { name: "Reset" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(resetSecrets).toHaveBeenCalled();
    expect(screen.queryByText("Try Again")).toBeNull();
  });

  it("keeps the sheet open with a message when the reset returns false", async () => {
    resetSecrets.mockResolvedValue(false);
    await mount();
    fireEvent.click(screen.getByText("Reset Keychain Item…"));
    fireEvent.click(screen.getByRole("button", { name: "Reset" }));
    await waitFor(() => expect(screen.getByText("The Keychain item couldn’t be reset.")).toBeTruthy());
    expect(screen.getByRole("alertdialog")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    fireEvent.click(screen.getByText("Reset Keychain Item…"));
    expect(screen.queryByText("The Keychain item couldn’t be reset.")).toBeNull();
  });

  it("closes the sheet and shows an error when the reset rejects", async () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => undefined);
    resetSecrets.mockRejectedValue("keychain write blocked");
    await mount();
    fireEvent.click(screen.getByText("Reset Keychain Item…"));
    fireEvent.click(screen.getByRole("button", { name: "Reset" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(screen.getByRole("alert").textContent).toContain("Vigia can't use the Keychain");
    log.mockRestore();
  });
});

describe("native toolbar while a sheet is open", () => {
  it("disables the toolbar while the Reset sheet shows and enables it after", async () => {
    getSettings.mockResolvedValue(makeView({ secrets_blocked: true }));
    await mount();
    screen.getByText("Reset Keychain Item…").focus();
    fireEvent.click(screen.getByText("Reset Keychain Item…"));
    expect(setSettingsToolbarEnabled).toHaveBeenLastCalledWith(false);
    expect(screen.getByRole("main").querySelector(".pane")?.hasAttribute("inert")).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(setSettingsToolbarEnabled).toHaveBeenLastCalledWith(true);
    expect(document.activeElement).toBe(screen.getByText("Reset Keychain Item…"));
  });

  it("remembers the pane in storage and survives storage that throws", async () => {
    await mount({ search: "/?pane=general" });
    expect(localStorage.getItem("vigia.settings.tab")).toBe("general");
    cleanup();
    const getItem = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    await mount();
    expect(screen.getByTestId("pane-accounts")).toBeTruthy();
    getItem.mockRestore();
  });
});

describe("useSettings", () => {
  it("needs a provider", () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => undefined);
    function Orphan() {
      useSettings();
      return null;
    }
    expect(() => render(<Orphan />)).toThrow(/SettingsContext provider/);
    log.mockRestore();
  });
});
