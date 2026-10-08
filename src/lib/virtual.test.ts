import { describe, expect, it } from "vitest";
import { visibleWindow } from "./virtual";

describe("visibleWindow", () => {
  it("renders nothing for an empty list", () => {
    expect(visibleWindow(0, 26, 0, 400, 5)).toEqual({ start: 0, end: 0 });
  });

  it("starts at the top with overscan below the viewport", () => {
    expect(visibleWindow(1000, 20, 0, 100, 3)).toEqual({ start: 0, end: 8 });
  });

  it("adds overscan on both sides while scrolled", () => {
    expect(visibleWindow(1000, 20, 400, 100, 3)).toEqual({ start: 17, end: 28 });
  });

  it("clamps to the end of the list", () => {
    expect(visibleWindow(30, 20, 5000, 100, 3)).toEqual({ start: 30, end: 30 });
    expect(visibleWindow(30, 20, 500, 100, 3)).toEqual({ start: 22, end: 30 });
  });

  it("renders every row when the list fits", () => {
    expect(visibleWindow(5, 20, 0, 400, 3)).toEqual({ start: 0, end: 5 });
  });

  it("includes a partly visible row", () => {
    expect(visibleWindow(100, 20, 10, 100, 0)).toEqual({ start: 0, end: 6 });
  });

  it("ignores negative scroll positions", () => {
    expect(visibleWindow(100, 20, -50, 100, 0)).toEqual({ start: 0, end: 5 });
  });
});
