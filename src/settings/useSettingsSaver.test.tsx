// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Settings } from "../lib/tauri";

const updateSettings = vi.fn();
vi.mock("../lib/tauri", () => ({
  updateSettings: (...args: unknown[]) => updateSettings(...args),
}));

import { useSettingsSaver } from "./useSettingsSaver";

const base: Settings = {
  poll_interval_secs: 60,
  exclude_pull_requests: false,
  branch_patterns: [],
  ignored_workflows: [],
  include_tags: false,
  notify_failures: true,
  notify_recoveries: true,
  launch_at_login: false,
  check_for_updates: true,
};

beforeEach(() => {
  updateSettings.mockReset();
  updateSettings.mockResolvedValue(undefined);
});
afterEach(cleanup);

describe("useSettingsSaver", () => {
  it("saves a patch merged onto the current settings and reloads", async () => {
    const onChange = vi.fn(async () => undefined);
    const onError = vi.fn();
    const { result } = renderHook(() => useSettingsSaver(base, onChange, onError));
    await act(() => result.current({ launch_at_login: true }));
    expect(updateSettings).toHaveBeenCalledWith({ ...base, launch_at_login: true });
    expect(onError).toHaveBeenCalledWith(null);
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("applies queued patches in order on top of each other", async () => {
    let release: () => void = () => undefined;
    updateSettings.mockImplementationOnce(() => new Promise<void>((r) => (release = r)));
    const { result } = renderHook(() => useSettingsSaver(base, async () => undefined, vi.fn()));
    let first!: Promise<void>;
    let second!: Promise<void>;
    await act(async () => {
      first = result.current({ poll_interval_secs: 30 });
      second = result.current({ notify_failures: false });
    });
    expect(updateSettings).toHaveBeenCalledTimes(1);
    await act(async () => {
      release();
      await first;
      await second;
    });
    expect(updateSettings).toHaveBeenNthCalledWith(2, { ...base, poll_interval_secs: 30, notify_failures: false });
  });

  it("reports a failure and drops the failed patch from later saves", async () => {
    updateSettings.mockRejectedValueOnce("server error: boom");
    const onChange = vi.fn(async () => undefined);
    const onError = vi.fn();
    const { result } = renderHook(() => useSettingsSaver(base, onChange, onError));
    await act(() => result.current({ poll_interval_secs: 15 }));
    expect(onError).toHaveBeenCalledWith("The server had a problem. Try again in a moment.");
    expect(onChange).not.toHaveBeenCalled();
    await act(() => result.current({ launch_at_login: true }));
    expect(updateSettings).toHaveBeenLastCalledWith({ ...base, launch_at_login: true });
  });

  it("refuses to save before settings are loaded", async () => {
    const onError = vi.fn();
    const { result } = renderHook(() => useSettingsSaver(null, async () => undefined, onError));
    await act(() => result.current({ launch_at_login: true }));
    expect(onError).toHaveBeenCalledWith("Settings are not loaded yet.");
    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("adopts settings that arrive after mount when nothing is saving", async () => {
    const onChange = vi.fn(async () => undefined);
    const { result, rerender } = renderHook(({ s }: { s: Settings | null }) => useSettingsSaver(s, onChange, vi.fn()), {
      initialProps: { s: null as Settings | null },
    });
    rerender({ s: { ...base, poll_interval_secs: 300 } });
    await act(() => result.current({ launch_at_login: true }));
    expect(updateSettings).toHaveBeenCalledWith({ ...base, poll_interval_secs: 300, launch_at_login: true });
  });
});
