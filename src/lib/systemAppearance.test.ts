// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

const getSystemAppearance = vi.fn();
vi.mock("./tauri", () => ({ getSystemAppearance: () => getSystemAppearance() }));

import { trackSystemAppearance, trackWindowActive } from "./systemAppearance";

function flush() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

afterEach(() => {
  delete (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__;
  document.documentElement.removeAttribute("class");
  document.documentElement.removeAttribute("style");
  vi.restoreAllMocks();
});

describe("trackWindowActive", () => {
  it("marks the root inactive while the window has no focus", () => {
    const focus = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const root = document.createElement("div");
    const stop = trackWindowActive(root);
    expect(root.classList.contains("window-inactive")).toBe(true);
    focus.mockReturnValue(true);
    window.dispatchEvent(new Event("focus"));
    expect(root.classList.contains("window-inactive")).toBe(false);
    focus.mockReturnValue(false);
    window.dispatchEvent(new Event("blur"));
    expect(root.classList.contains("window-inactive")).toBe(true);
    stop();
  });
});

describe("trackSystemAppearance", () => {
  function stubMatchMedia() {
    // jsdom has no matchMedia.
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: () => ({ addEventListener: vi.fn(), removeEventListener: vi.fn() }),
    });
  }

  it("sets the accent and Reduce Transparency, and reads them again on focus", async () => {
    stubMatchMedia();
    getSystemAppearance.mockResolvedValueOnce({ accent: "#ff2d55", reduce_transparency: true });
    const root = document.createElement("div");
    const stop = trackSystemAppearance(root);
    await flush();
    expect(root.style.getPropertyValue("--accent")).toBe("#ff2d55");
    expect(root.classList.contains("reduce-transparency")).toBe(true);
    getSystemAppearance.mockResolvedValueOnce({ accent: null, reduce_transparency: false });
    window.dispatchEvent(new Event("focus"));
    await flush();
    expect(root.style.getPropertyValue("--accent")).toBe("");
    expect(root.classList.contains("reduce-transparency")).toBe(false);
    stop();
  });

  it("keeps the stylesheet values when nothing is reported or the read fails", async () => {
    stubMatchMedia();
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    getSystemAppearance.mockResolvedValueOnce(null);
    const root = document.createElement("div");
    const stop = trackSystemAppearance(root);
    await flush();
    expect(root.getAttribute("style")).toBeNull();
    getSystemAppearance.mockRejectedValueOnce(new Error("no backend"));
    window.dispatchEvent(new Event("focus"));
    await flush();
    expect(error).toHaveBeenCalled();
    stop();
  });

  it("reads nothing in the browser preview", () => {
    (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__ = true;
    getSystemAppearance.mockClear();
    trackSystemAppearance(document.createElement("div"))();
    expect(getSystemAppearance).not.toHaveBeenCalled();
  });
});
