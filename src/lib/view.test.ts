import { describe, expect, it } from "vitest";
import { viewFromSearch } from "./view";

describe("viewFromSearch", () => {
  it("returns popup when the parameter is missing", () => {
    expect(viewFromSearch("")).toBe("popup");
  });

  it("returns settings for view=settings", () => {
    expect(viewFromSearch("?view=settings")).toBe("settings");
  });

  it("falls back to popup for unknown values", () => {
    expect(viewFromSearch("?view=other")).toBe("popup");
  });
});
