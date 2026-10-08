import type { AccountKind, AccountSnapshot } from "../lib/snapshot";
import type { Account } from "../lib/tauri";

export type StatusColor = "green" | "red" | "orange" | "gray";

/** An account's connection state: the dot colour and the text that names it. */
export interface AccountStatus {
  color: StatusColor;
  text: string;
}

/** Describes an account's connection state from its snapshot, or "Checking…" before the first one. */
export function accountStatus(status: AccountSnapshot | undefined): AccountStatus {
  if (!status) {
    return { color: "gray", text: "Checking…" };
  }
  if (status.auth_error) {
    return { color: "red", text: status.keychain_denied ? "Can’t read Keychain" : "Token rejected" };
  }
  if (status.unreachable) {
    return { color: "orange", text: "Unreachable" };
  }
  if (status.rate_limited_until) {
    return { color: "orange", text: "Rate limited" };
  }
  return { color: "green", text: "Connected" };
}

/** Whether a server address sends the token unencrypted. */
export function isInsecureUrl(url: string | undefined): boolean {
  return (url ?? "").trim().toLowerCase().startsWith("http://");
}

/** The host an account connects to, or its raw address when that is not a URL. */
export function hostOf(account: Account): string {
  if (!account.base_url) {
    return "github.com";
  }
  try {
    return new URL(account.base_url).host;
  } catch {
    return account.base_url;
  }
}

/** The name a new account gets when none is typed: the organization, or the server's host. */
export function defaultLabel(kind: AccountKind, owner: string, baseUrl: string): string {
  if (kind === "github") {
    return owner.trim() || "GitHub";
  }
  try {
    return new URL(baseUrl.trim()).host || "GitLab";
  } catch {
    return "GitLab";
  }
}

export function providerName(kind: AccountKind): string {
  return kind === "github" ? "GitHub" : "GitLab";
}

export function pluralize(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}
