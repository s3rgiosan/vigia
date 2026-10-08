import { describe, expect, it } from "vitest";
import { authPopupMessage, authReason, authSettingsMessage, authStatusText } from "./auth";
import type { AuthReason } from "./snapshot";

describe("authReason", () => {
  it("returns the reported reason", () => {
    expect(authReason({ auth_reason: "missing_token" })).toBe("missing_token");
    expect(authReason({ auth_reason: "keychain" })).toBe("keychain");
    expect(authReason({ auth_reason: "rejected" })).toBe("rejected");
  });

  it("treats a missing or unknown reason as a rejected token", () => {
    expect(authReason({ auth_reason: null })).toBe("rejected");
    expect(authReason({} as { auth_reason: null })).toBe("rejected");
    expect(authReason({ auth_reason: "other" as AuthReason })).toBe("rejected");
  });
});

describe("auth wording", () => {
  const cases: { reason: AuthReason; status: string; settings: string; popup: string }[] = [
    {
      reason: "rejected",
      status: "Token rejected",
      settings: "The token was rejected. Replace it to resume checking.",
      popup: "The token for “Acme” was rejected.",
    },
    {
      reason: "missing_token",
      status: "No token saved",
      settings: "No token is saved for this account. Add one with Replace Token… to start checking.",
      popup: "No token is saved for “Acme”.",
    },
    {
      reason: "keychain",
      status: "Can’t read Keychain",
      settings: "Vigia can’t read its tokens from the Keychain.",
      popup: "Vigia can’t read the token for “Acme” from the Keychain.",
    },
  ];

  for (const c of cases) {
    it(`words a ${c.reason} account`, () => {
      expect(authStatusText(c.reason)).toBe(c.status);
      expect(authSettingsMessage(c.reason)).toBe(c.settings);
      expect(authPopupMessage(c.reason, "Acme")).toBe(c.popup);
    });
  }
});
