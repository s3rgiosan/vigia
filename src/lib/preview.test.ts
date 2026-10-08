// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { isPreview } from "./preview";

afterEach(() => {
  delete (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__;
});

describe("isPreview", () => {
  it("reads the flag at call time", () => {
    expect(isPreview()).toBe(false);
    (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__ = true;
    expect(isPreview()).toBe(true);
  });
});
