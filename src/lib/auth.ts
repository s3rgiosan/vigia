import type { AccountSnapshot, AuthReason } from "./snapshot";

/** The cause of an account's auth error; a snapshot without a known reason counts as a rejected token. */
export function authReason(account: Pick<AccountSnapshot, "auth_reason">): AuthReason {
  switch (account.auth_reason) {
    case "missing_token":
    case "keychain":
      return account.auth_reason;
    default:
      return "rejected";
  }
}

/** The short status label for an account with an auth error. */
export function authStatusText(reason: AuthReason): string {
  switch (reason) {
    case "missing_token":
      return "No token saved";
    case "keychain":
      return "Can’t read Keychain";
    default:
      return "Token rejected";
  }
}

/** The banner text in the account's Settings pane. */
export function authSettingsMessage(reason: AuthReason): string {
  switch (reason) {
    case "missing_token":
      return "No token is saved for this account. Add one with Replace Token… to start checking.";
    case "keychain":
      return "Vigia can’t read its tokens from the Keychain.";
    default:
      return "The token was rejected. Replace it to resume checking.";
  }
}

/** The popup banner text for an account with an auth error. */
export function authPopupMessage(reason: AuthReason, label: string): string {
  switch (reason) {
    case "missing_token":
      return `No token is saved for “${label}”.`;
    case "keychain":
      return `Vigia can’t read the token for “${label}” from the Keychain.`;
    default:
      return `The token for “${label}” was rejected.`;
  }
}
