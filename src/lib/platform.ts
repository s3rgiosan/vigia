// The Tauri CLI sets TAURI_ENV_PLATFORM for the frontend build: `darwin`, `linux` or `windows`.
// The browser preview and the tests have none and follow macOS.
const PLATFORM: string = import.meta.env.TAURI_ENV_PLATFORM ?? "darwin";

/** Whether this build runs on macOS, where Settings has a native toolbar. */
export const IS_MACOS = PLATFORM === "darwin";

/** How the OS names the store that holds Vigia's tokens, and what to say when it refuses access. */
export interface SecretStoreNames {
  /** The store, as it reads after "the": "Keychain". */
  store: string;
  /** Vigia's single item in it, in sentence case: "Keychain item". */
  item: string;
  /** The item in title case, for buttons and sheet titles: "Keychain Item". */
  itemTitle: string;
  /** What the user can do when access is denied, as a sentence. */
  deniedHint: string;
  /** Where the OS trusts certificate authorities, as it reads after "in": "the macOS Keychain". */
  trustStore: string;
}

export function secretStoreNames(platform: string): SecretStoreNames {
  switch (platform) {
    case "windows":
      return {
        store: "Credential Manager",
        item: "credential",
        itemTitle: "Credential",
        deniedHint: "Reset the credential and add the tokens again.",
        trustStore: "the Windows certificate store",
      };
    case "linux":
      return {
        store: "keyring",
        item: "keyring item",
        itemTitle: "Keyring Item",
        deniedHint:
          "Check that a keyring service such as GNOME Keyring or KWallet is running and unlocked, or reset the keyring item.",
        trustStore: "the system certificate store",
      };
    default:
      return {
        store: "Keychain",
        item: "Keychain item",
        itemTitle: "Keychain Item",
        deniedHint: "Allow access when macOS asks, or reset the Keychain item.",
        trustStore: "the macOS Keychain",
      };
  }
}

export const SECRET_STORE = secretStoreNames(PLATFORM);
