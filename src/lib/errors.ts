import { SECRET_STORE } from "./platform";

/** Longest raw backend text echoed back when no friendlier wording applies. */
const MAX_RAW = 120;

function rawText(e: unknown): string {
  if (e instanceof Error) {
    return e.message;
  }
  if (typeof e === "string") {
    return e;
  }
  try {
    return String(e);
  } catch {
    return "";
  }
}

export type ErrorKind =
  | "unauthorized"
  | "forbidden"
  | "not_found"
  | "rate_limited"
  | "network"
  | "server"
  | "redirected"
  | "decode"
  | "keychain"
  | "read_only"
  | "invalid_input"
  | "unknown_account"
  | "internal";

interface KindedError {
  kind: string;
  message: string;
}

function isKindedError(e: unknown): e is KindedError {
  if (typeof e !== "object" || e === null) {
    return false;
  }
  const candidate = e as Record<string, unknown>;
  return typeof candidate.kind === "string" && typeof candidate.message === "string";
}

function kindMessage(e: KindedError): string | null {
  switch (e.kind) {
    case "unauthorized":
      return "The server rejected this token. Check it's copied in full and hasn't expired.";
    case "forbidden":
      return "The token doesn't have access to that. Check its repository permissions.";
    case "not_found":
      return "The server couldn't find that. Check the address and that the token can see it.";
    case "rate_limited":
      return "The server is limiting requests. Vigia will try again shortly.";
    case "network":
      return "Can't reach the server. Check the address and your connection.";
    case "server":
      return "The server had a problem. Try again in a moment.";
    case "redirected":
      return "The server moved to another address. Enter its current address and try again.";
    case "decode":
      return "The server sent a reply Vigia doesn't understand. Check that the address is a GitHub or GitLab server.";
    case "keychain":
      return `Vigia can’t read its tokens from the ${SECRET_STORE.store}.`;
    case "read_only":
      return "Settings were saved by a newer version of Vigia, so changes can’t be saved.";
    case "invalid_input":
      return e.message.trim() === "" ? null : e.message.trim();
    case "unknown_account":
      return "That account no longer exists. Reopen Settings and try again.";
    case "internal":
    default:
      return null;
  }
}

/**
 * Turns an error returned by the backend into a plain, actionable sentence. Unrecognised errors
 * keep a shortened copy of the original text so the cause is not lost.
 */
export function friendlyError(e: unknown): string {
  if (isKindedError(e)) {
    const known = kindMessage(e);
    if (known !== null) {
      return known;
    }
    return friendlyText(e.message);
  }
  return friendlyText(rawText(e));
}

function friendlyText(message: string): string {
  const raw = message.trim();
  const text = raw.toLowerCase();

  if (text === "token rejected") {
    return "The server rejected this token. Check it's copied in full and hasn't expired.";
  }
  if (text === "rate limited") {
    return "The server is limiting requests. Vigia will try again shortly.";
  }
  if (text === "not found") {
    return "The server couldn't find that. Check the address and that the token can see it.";
  }
  if (text === "no access") {
    return "The token doesn't have access to that. Check its repository permissions.";
  }
  if (text.startsWith("network error")) {
    return "Can't reach the server. Check the address and your connection.";
  }
  if (text.startsWith("the server redirected to")) {
    return "The server moved to another address. Enter its current address and try again.";
  }
  if (text.startsWith("server error")) {
    return "The server had a problem. Try again in a moment.";
  }
  if (text.startsWith("unexpected response")) {
    return "The server sent a reply Vigia doesn't understand. Check that the address is a GitHub or GitLab server.";
  }
  if (text.startsWith("keychain access denied") || text.startsWith("keychain write blocked")) {
    return `Vigia can't use the ${SECRET_STORE.store}. ${SECRET_STORE.deniedHint}`;
  }
  if (text.startsWith("stored tokens are not valid")) {
    return `The tokens stored in the ${SECRET_STORE.store} are unreadable. Reset the ${SECRET_STORE.item} and add them again.`;
  }
  if (text.startsWith("the config was written by a newer")) {
    return "These settings were saved by a newer version of Vigia and can't be changed here.";
  }
  if (text.startsWith("could not read or write the config file")) {
    return "Vigia can't save its settings file. Check the disk and folder permissions.";
  }
  if (text.startsWith("the config file is not valid")) {
    return "The settings file is damaged, so changes can't be saved.";
  }
  if (text === "unknown account") {
    return "That account no longer exists. Reopen Settings and try again.";
  }
  if (text.startsWith("this repo is already watched through")) {
    return "This repository is already watched through another account.";
  }
  if (text.startsWith("the new base url points at a different")) {
    return "That address belongs to a different server or user. Add it as a new account instead.";
  }
  if (text.startsWith("vigia needs a fine-grained github token")) {
    return "Use a fine-grained token that starts with github_pat_. Classic tokens aren't accepted.";
  }
  if (text.includes("base url is required")) {
    return "Enter the server address.";
  }
  if (text.includes("http:// base url")) {
    return "This address uses http://, which sends the token unencrypted. Confirm to continue, or use https://.";
  }
  if (raw === "") {
    return "Something went wrong.";
  }
  const short = raw.length > MAX_RAW ? `${raw.slice(0, MAX_RAW - 1)}…` : raw;
  return `Something went wrong: ${short}`;
}
