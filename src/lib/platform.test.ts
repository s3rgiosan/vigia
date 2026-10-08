import { describe, expect, it } from "vitest";
import { IS_MACOS, SECRET_STORE, secretStoreNames } from "./platform";

describe("platform", () => {
  it("follows macOS when no platform is set", () => {
    expect(IS_MACOS).toBe(true);
    expect(SECRET_STORE).toEqual(secretStoreNames("darwin"));
  });
});

describe("secretStoreNames", () => {
  it("names the Keychain on macOS", () => {
    const names = secretStoreNames("darwin");
    expect(names.store).toBe("Keychain");
    expect(names.itemTitle).toBe("Keychain Item");
    expect(names.deniedHint).toContain("macOS");
    expect(names.trustStore).toBe("the macOS Keychain");
  });

  it("names Credential Manager on Windows", () => {
    const names = secretStoreNames("windows");
    expect(names.store).toBe("Credential Manager");
    expect(names.item).toBe("credential");
    expect(names.itemTitle).toBe("Credential");
    expect(names.trustStore).toBe("the Windows certificate store");
  });

  it("names the keyring on Linux and points at the keyring service", () => {
    const names = secretStoreNames("linux");
    expect(names.store).toBe("keyring");
    expect(names.item).toBe("keyring item");
    expect(names.deniedHint).toContain("GNOME Keyring");
    expect(names.trustStore).toBe("the system certificate store");
  });

  it("uses the macOS names for an unknown platform", () => {
    expect(secretStoreNames("")).toEqual(secretStoreNames("darwin"));
  });
});
