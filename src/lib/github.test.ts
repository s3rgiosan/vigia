import { describe, expect, it } from "vitest";
import { tokenTypeError } from "./github";

describe("tokenTypeError", () => {
  it("accepts fine-grained tokens and empty input", () => {
    expect(tokenTypeError("")).toBeNull();
    expect(tokenTypeError("  github_pat_11ABC ")).toBeNull();
  });

  it("explains classic and other tokens", () => {
    expect(tokenTypeError("ghp_abc")).toMatch(/classic token/);
    expect(tokenTypeError("gho_abc")).toMatch(/github_pat_/);
  });
});
