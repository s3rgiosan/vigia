import { describe, expect, it } from "vitest";
import { friendlyError } from "./errors";

describe("friendlyError", () => {
  it("explains a rejected token", () => {
    expect(friendlyError("token rejected")).toBe(
      "The server rejected this token. Check it's copied in full and hasn't expired.",
    );
  });

  it("explains network failures whatever the detail", () => {
    expect(friendlyError("network error: dns error: failed to lookup address")).toBe(
      "Can't reach the server. Check the address and your connection.",
    );
  });

  it("maps provider statuses", () => {
    expect(friendlyError("rate limited")).toMatch(/limiting requests/);
    expect(friendlyError("not found")).toMatch(/couldn't find/);
    expect(friendlyError("no access")).toMatch(/doesn't have access/);
    expect(friendlyError("server error 502")).toMatch(/server had a problem/);
    expect(friendlyError("the server redirected to https://example.test/; update the base URL")).toMatch(/moved/);
  });

  it("maps keychain and config failures", () => {
    expect(friendlyError("Keychain access denied")).toMatch(/Keychain/);
    expect(friendlyError("the config was written by a newer Vigia and is read-only")).toMatch(/newer version/);
  });

  it("accepts Error objects", () => {
    expect(friendlyError(new Error("token rejected"))).toMatch(/rejected this token/);
  });

  it("keeps a short copy of unknown errors", () => {
    expect(friendlyError("boom")).toBe("Something went wrong: boom");
    const long = friendlyError("x".repeat(500));
    expect(long.length).toBeLessThan(150);
    expect(long.endsWith("…")).toBe(true);
  });

  it("handles empty errors", () => {
    expect(friendlyError("")).toBe("Something went wrong.");
  });
});

describe("friendlyError mappings", () => {
  const cases: [string, RegExp][] = [
    ["Stored tokens are not valid JSON", /unreadable/],
    ["Keychain write blocked", /can't use the Keychain/],
    ["could not read or write the config file: denied", /settings file/],
    ["the config file is not valid TOML", /damaged/],
    ["unknown account", /no longer exists/],
    ["this repo is already watched through Work", /another account/],
    ["the new base URL points at a different host", /different server/],
    ["Vigia needs a fine-grained GitHub token", /github_pat_/],
    ["a base URL is required", /Enter the server address/],
    ["an http:// base URL needs confirmation", /unencrypted/],
    ["unexpected response from server", /doesn't understand/],
  ];

  it.each(cases)("maps %s", (input, expected) => {
    expect(friendlyError(input)).toMatch(expected);
  });

  it("ignores case and surrounding whitespace", () => {
    expect(friendlyError("  RATE LIMITED \n")).toMatch(/limiting requests/);
  });

  it("stringifies other values", () => {
    expect(friendlyError(42)).toBe("Something went wrong: 42");
    expect(friendlyError(null)).toBe("Something went wrong: null");
  });

  it("falls back to the generic text when the value cannot be stringified", () => {
    const hostile = {
      toString() {
        throw new Error("no string form");
      },
    };
    expect(friendlyError(hostile)).toBe("Something went wrong.");
  });

  it("keeps text of exactly the maximum length untouched", () => {
    const text = "y".repeat(120);
    expect(friendlyError(text)).toBe(`Something went wrong: ${text}`);
  });
});

describe("structured errors", () => {
  const kinds: [string, RegExp][] = [
    ["unauthorized", /rejected this token/],
    ["forbidden", /doesn't have access/],
    ["not_found", /couldn't find/],
    ["rate_limited", /limiting requests/],
    ["network", /Can't reach the server/],
    ["server", /server had a problem/],
    ["redirected", /moved to another address/],
    ["decode", /doesn't understand/],
    ["keychain", /Vigia can’t read its tokens from the Keychain\./],
    ["read_only", /newer version of Vigia, so changes can’t be saved\./],
    ["unknown_account", /no longer exists/],
  ];

  it.each(kinds)("words %s", (kind, pattern) => {
    expect(friendlyError({ kind, message: "raw detail" })).toMatch(pattern);
  });

  it("uses the message for invalid input", () => {
    expect(friendlyError({ kind: "invalid_input", message: "  Enter the server address.  " })).toBe(
      "Enter the server address.",
    );
  });

  it("falls back to the message text for an empty invalid input, internal and unknown kinds", () => {
    expect(friendlyError({ kind: "invalid_input", message: "" })).toBe("Something went wrong.");
    expect(friendlyError({ kind: "internal", message: "boom" })).toBe("Something went wrong: boom");
    expect(friendlyError({ kind: "future_kind", message: "token rejected" })).toMatch(/rejected this token/);
  });
});
