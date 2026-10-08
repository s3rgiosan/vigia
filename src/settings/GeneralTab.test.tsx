// @vitest-environment jsdom
import { act, cleanup, fireEvent, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { getVersion } from "@tauri-apps/api/app";
import { checkForUpdatesNow, installUpdate, onUpdateProgress, type Settings, type UpdateProgress } from "../lib/tauri";
import { makeSettings, makeView } from "../test/fixtures";
import { renderWithSettings } from "../test/settings";

vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn() }));
vi.mock("../lib/tauri", () => ({
  checkForUpdatesNow: vi.fn(),
  installUpdate: vi.fn(),
  onUpdateProgress: vi.fn(),
  subscribe: vi.fn(() => () => undefined),
}));

import { GeneralTab } from "./GeneralTab";

let pushProgress: (p: UpdateProgress) => void;

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => undefined);
  vi.mocked(onUpdateProgress).mockImplementation(((handler: (p: UpdateProgress) => void) => {
    pushProgress = handler;
    return Promise.resolve(() => undefined);
  }) as unknown as typeof onUpdateProgress);
  vi.mocked(installUpdate).mockResolvedValue(undefined);
  vi.mocked(getVersion).mockResolvedValue("1.0.0");
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function renderTab(patch: Partial<Settings> = {}, readOnly = false) {
  const { context } = renderWithSettings(<GeneralTab />, {
    view: makeView({ settings: makeSettings(patch), read_only: readOnly }),
  });
  return context.saveSettings as Mock;
}

describe("general settings", () => {
  it("saves the chosen check interval", () => {
    const save = renderTab();
    fireEvent.change(screen.getByLabelText("Check every"), { target: { value: "300" } });
    expect(save).toHaveBeenCalledWith({ poll_interval_secs: 300 });
  });

  it("adds a custom interval to the list in order", () => {
    renderTab({ poll_interval_secs: 45 });
    const options = Array.from(screen.getByLabelText("Check every").querySelectorAll("option")).map((o) => o.textContent);
    expect(options).toEqual(["15 seconds", "30 seconds", "45 seconds", "1 minute", "2 minutes", "5 minutes", "10 minutes"]);
  });

  it("saves each toggle", () => {
    const save = renderTab();
    fireEvent.click(screen.getByLabelText("Ignore pull request runs"));
    fireEvent.click(screen.getByLabelText("Notify when a branch fails"));
    fireEvent.click(screen.getByLabelText("Notify when a branch recovers"));
    fireEvent.click(screen.getByLabelText("Open Vigia at login"));
    expect(save.mock.calls).toEqual([
      [{ exclude_pull_requests: true }],
      [{ notify_failures: false }],
      [{ notify_recoveries: false }],
      [{ launch_at_login: true }],
    ]);
  });

  it("saves the automatic update switch", () => {
    const save = renderTab();
    fireEvent.click(screen.getByLabelText("Check for updates automatically"));
    expect(save).toHaveBeenCalledWith({ check_for_updates: false });
  });

  it("disables recovery notifications while failure notifications are off", () => {
    renderTab({ notify_failures: false });
    expect((screen.getByLabelText("Notify when a branch recovers") as HTMLInputElement).disabled).toBe(true);
  });

  it("disables every control when the config is read-only", () => {
    renderTab({}, true);
    for (const el of document.querySelectorAll("input, select")) {
      expect((el as HTMLInputElement).disabled).toBe(true);
    }
  });

  describe("Check Now", () => {
    const checkNow = () => screen.getByRole("button", { name: "Check Now" });

    it("reports an up to date app", async () => {
      let resolve: (v: null) => void = () => undefined;
      vi.mocked(checkForUpdatesNow).mockReturnValue(new Promise((r) => (resolve = r)));
      renderTab();
      fireEvent.click(checkNow());
      expect(screen.getByText("Checking…")).toBeTruthy();
      expect((checkNow() as HTMLButtonElement).disabled).toBe(true);
      await act(async () => resolve(null));
      expect(screen.getByText("Up to date")).toBeTruthy();
    });

    it("reports a failed check", async () => {
      vi.mocked(checkForUpdatesNow).mockRejectedValue({ kind: "network", message: "offline" });
      renderTab();
      fireEvent.click(checkNow());
      await waitFor(() => expect(screen.getByText("Couldn’t check for updates")).toBeTruthy());
    });

    it("offers the install and follows its progress", async () => {
      vi.mocked(checkForUpdatesNow).mockResolvedValue({ version: "2.0.0", notes: null });
      renderTab();
      fireEvent.click(checkNow());
      await waitFor(() => expect(screen.getByText("Version 2.0.0 available")).toBeTruthy());
      fireEvent.click(screen.getByRole("button", { name: "Install and Relaunch" }));
      expect(installUpdate).toHaveBeenCalled();
      act(() => pushProgress({ downloaded: 1, total: 4 }));
      expect(screen.getByRole("button", { name: "Downloading… 25%" })).toBeTruthy();
    });

    it("reports a failed install", async () => {
      vi.mocked(checkForUpdatesNow).mockResolvedValue({ version: "2.0.0", notes: null });
      vi.mocked(installUpdate).mockRejectedValue({ kind: "network", message: "offline" });
      renderTab();
      fireEvent.click(checkNow());
      await waitFor(() => screen.getByText("Version 2.0.0 available"));
      fireEvent.click(screen.getByRole("button", { name: "Install and Relaunch" }));
      await waitFor(() => expect(screen.getByText("Couldn’t install the update")).toBeTruthy());
      expect(screen.queryByText("Couldn’t check for updates")).toBeNull();
    });
  });

  describe("Version", () => {
    it("shows the app version", async () => {
      renderTab();
      await waitFor(() => expect(screen.getByText("1.0.0")).toBeTruthy());
    });

    it("leaves the value empty when the version cannot be read", async () => {
      vi.mocked(getVersion).mockRejectedValue(new Error("denied"));
      renderTab();
      await act(async () => undefined);
      expect(screen.getByText("Version").closest(".form-row")?.querySelector(".value")?.textContent).toBe("");
    });
  });
});
