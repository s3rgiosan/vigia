// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useDocumentVisible } from "./useDocumentVisible";
import { useLatest } from "./useLatest";
import { useRovingListbox } from "./useRovingListbox";
import { useSafeStorage } from "./useSafeStorage";

afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("useLatest", () => {
  it("holds the value of the latest render", () => {
    const { result, rerender } = renderHook(({ v }) => useLatest(v), {
      initialProps: { v: 1 },
    });
    expect(result.current.current).toBe(1);
    rerender({ v: 2 });
    expect(result.current.current).toBe(2);
  });
});

describe("useRovingListbox", () => {
  function setup(overrides: Partial<Parameters<typeof useRovingListbox>[0]> = {}) {
    const onMove = vi.fn();
    const onActivate = vi.fn();
    const { result } = renderHook(() =>
      useRovingListbox({
        count: 25,
        activeIndex: 12,
        onMove,
        onActivate,
        pageSize: 5,
        ...overrides,
      }),
    );
    const press = (key: string) => {
      const preventDefault = vi.fn();
      result.current({ key, preventDefault });
      return preventDefault;
    };
    return { onMove, onActivate, press };
  }

  it.each([
    ["ArrowDown", 13],
    ["ArrowUp", 11],
    ["Home", 0],
    ["End", 24],
    ["PageDown", 17],
    ["PageUp", 7],
  ])("%s moves to %i and prevents the default", (key, index) => {
    const { onMove, press } = setup();
    const prevented = press(key);
    expect(onMove).toHaveBeenCalledWith(index);
    expect(prevented).toHaveBeenCalled();
  });

  it("clamps at both ends and defaults the page size to 10", () => {
    const first = setup({ activeIndex: 0 });
    first.press("ArrowUp");
    first.press("PageUp");
    expect(first.onMove).toHaveBeenNthCalledWith(1, 0);
    expect(first.onMove).toHaveBeenNthCalledWith(2, 0);
    const last = setup({ activeIndex: 24, pageSize: undefined });
    last.press("ArrowDown");
    expect(last.onMove).toHaveBeenCalledWith(24);
    const mid = setup({ pageSize: undefined });
    mid.press("PageDown");
    expect(mid.onMove).toHaveBeenCalledWith(22);
  });

  it("activates the active option on Space and Enter", () => {
    const { onActivate, press } = setup();
    press(" ");
    press("Enter");
    expect(onActivate).toHaveBeenCalledTimes(2);
    expect(onActivate).toHaveBeenCalledWith(12);
  });

  it("leaves Space and Enter alone without an activate handler", () => {
    const { press } = setup({ onActivate: undefined });
    expect(press("Enter")).not.toHaveBeenCalled();
  });

  it("ignores other keys and empty lists", () => {
    const { onMove, press } = setup();
    expect(press("a")).not.toHaveBeenCalled();
    const empty = setup({ count: 0 });
    expect(empty.press("ArrowDown")).not.toHaveBeenCalled();
    expect(onMove).not.toHaveBeenCalled();
    expect(empty.onMove).not.toHaveBeenCalled();
  });

  it("uses the latest options without changing the handler", () => {
    const onMove = vi.fn();
    const { result, rerender } = renderHook(({ i }) => useRovingListbox({ count: 5, activeIndex: i, onMove }), {
      initialProps: { i: 0 },
    });
    const handler = result.current;
    rerender({ i: 2 });
    expect(result.current).toBe(handler);
    handler({ key: "ArrowDown", preventDefault: () => undefined });
    expect(onMove).toHaveBeenCalledWith(3);
  });
});

describe("useSafeStorage", () => {
  it("returns the fallback and persists updates", () => {
    const { result } = renderHook(() => useSafeStorage("k", "dflt"));
    expect(result.current[0]).toBe("dflt");
    act(() => result.current[1]("next"));
    expect(result.current[0]).toBe("next");
    expect(localStorage.getItem("k")).toBe("next");
  });

  it("reads a stored value", () => {
    localStorage.setItem("k", "saved");
    const { result } = renderHook(() => useSafeStorage("k", "dflt"));
    expect(result.current[0]).toBe("saved");
  });

  it("survives storage that throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const { result } = renderHook(() => useSafeStorage("k", "dflt"));
    expect(result.current[0]).toBe("dflt");
    act(() => result.current[1]("mem"));
    expect(result.current[0]).toBe("mem");
  });
});

describe("useDocumentVisible", () => {
  function setState(visibility: DocumentVisibilityState, focused: boolean) {
    vi.spyOn(document, "visibilityState", "get").mockReturnValue(visibility);
    vi.spyOn(document, "hasFocus").mockReturnValue(focused);
  }

  it("follows visibility and focus", () => {
    setState("visible", true);
    const { result } = renderHook(() => useDocumentVisible());
    expect(result.current).toBe(true);

    setState("hidden", true);
    act(() => void document.dispatchEvent(new Event("visibilitychange")));
    expect(result.current).toBe(false);

    setState("visible", false);
    act(() => void window.dispatchEvent(new Event("blur")));
    expect(result.current).toBe(false);

    setState("visible", true);
    act(() => void window.dispatchEvent(new Event("focus")));
    expect(result.current).toBe(true);
  });

  it("stops listening after unmount", () => {
    setState("visible", true);
    const { result, unmount } = renderHook(() => useDocumentVisible());
    unmount();
    setState("hidden", false);
    act(() => void window.dispatchEvent(new Event("blur")));
    expect(result.current).toBe(true);
  });
});
