import { authPopupMessage, authReason } from "./auth";
import type { AccountSnapshot, RepoSnapshot, RepoStatus, Run, RunState, Snapshot, TrayColor } from "./snapshot";

export type SectionId = "failed" | "error" | "running" | "passing" | "none";

export interface Section {
  id: SectionId;
  title: string;
  /** Expanded by default; Passing and No runs start collapsed. */
  open: boolean;
  groups: OrgGroup[];
  count: number;
}

export interface OrgGroup {
  account: AccountSnapshot;
  /** Org or namespace, the part of the full name before the last slash. */
  org: string;
  repos: RepoSnapshot[];
}

const SECTION_ORDER: { id: SectionId; title: string; open: boolean; statuses: RepoStatus[] }[] = [
  { id: "failed", title: "Failed", open: true, statuses: ["failed"] },
  { id: "error", title: "Errors", open: true, statuses: ["error"] },
  { id: "running", title: "Running", open: true, statuses: ["running"] },
  { id: "passing", title: "Passing", open: false, statuses: ["success"] },
  { id: "none", title: "No runs", open: false, statuses: ["none"] },
];

/** Org or namespace of a repo: everything before the last slash. */
export function orgOf(fullName: string): string {
  const index = fullName.lastIndexOf("/");
  return index === -1 ? fullName : fullName.slice(0, index);
}

/** Case-insensitive match on the repo name, branch, or run name. */
export function matchesFilter(repo: RepoSnapshot, filter: string): boolean {
  const needle = filter.trim().toLowerCase();
  if (needle === "") {
    return true;
  }
  const run = repo.state.representative;
  const haystack = [repo.repo.full_name, run?.branch ?? "", run?.name ?? ""].join(" ").toLowerCase();
  return haystack.includes(needle);
}

/** Splits repositories into the five sections, each grouped by account and then org. */
export function buildSections(snapshot: Snapshot, filter: string): Section[] {
  const accounts = new Map(snapshot.accounts.map((a) => [a.id, a]));
  const unknownAccount = (id: string): AccountSnapshot => ({
    id,
    label: "Unknown account",
    kind: "github",
    auth_error: false,
    auth_reason: null,
    unreachable: false,
    rate_limited_until: null,
    effective_interval_secs: 0,
    configured_interval_secs: 0,
    keychain_denied: false,
    error: null,
  });

  return SECTION_ORDER.map(({ id, title, open, statuses }) => {
    const repos = snapshot.repos.filter(
      (r) => statuses.includes(r.state.status) && matchesFilter(r, filter),
    );
    const byKey = new Map<string, OrgGroup>();
    for (const repo of repos) {
      const account = accounts.get(repo.account_id) ?? unknownAccount(repo.account_id);
      const org = orgOf(repo.repo.full_name);
      const key = `${account.id}\u0000${org}`;
      const group = byKey.get(key) ?? { account, org, repos: [] };
      group.repos.push(repo);
      byKey.set(key, group);
    }
    const groups = [...byKey.values()].sort(
      (a, b) => a.account.label.localeCompare(b.account.label) || a.org.localeCompare(b.org),
    );
    for (const group of groups) {
      group.repos.sort((a, b) =>
        a.repo.full_name.localeCompare(b.repo.full_name, undefined, { numeric: true, sensitivity: "base" }),
      );
    }
    return { id, title, open, groups, count: repos.length };
  });
}

/** Relative time such as "3m ago", for run timestamps. */
export function relativeTime(iso: string, now: Date = new Date()): string {
  const elapsed = now.getTime() - new Date(iso).getTime();
  if (Number.isNaN(elapsed)) {
    return "";
  }
  const seconds = Math.max(0, Math.round(elapsed / 1000));
  if (seconds < 60) {
    return "just now";
  }
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m ago`;
  }
  const hours = Math.round(minutes / 60);
  if (hours < 48) {
    return `${hours}h ago`;
  }
  const days = Math.round(hours / 24);
  return `${days}d ago`;
}

export type Banner =
  | { kind: "secrets_blocked" }
  | { kind: "config_read_only" }
  | { kind: "config_error"; message: string }
  | { kind: "auth"; account: AccountSnapshot }
  | { kind: "unreachable"; account: AccountSnapshot }
  | { kind: "rate_limited"; account: AccountSnapshot; until: number };

/** Banners to show above the list, one per affected account. */
export function buildBanners(snapshot: Snapshot, nowUnix: number): Banner[] {
  const banners: Banner[] = [];
  if (snapshot.secrets_blocked) {
    banners.push({ kind: "secrets_blocked" });
  }
  if (snapshot.config_error !== null) {
    banners.push({ kind: "config_error", message: snapshot.config_error });
  }
  if (snapshot.config_read_only && snapshot.config_error === null) {
    banners.push({ kind: "config_read_only" });
  }
  for (const account of snapshot.accounts) {
    if (account.auth_error) {
      // The Keychain banner already covers accounts whose token is unreadable.
      const coveredByKeychainBanner = snapshot.secrets_blocked && authReason(account) === "keychain";
      if (!coveredByKeychainBanner) {
        banners.push({ kind: "auth", account });
      }
    } else if (account.unreachable) {
      banners.push({ kind: "unreachable", account });
    } else if (account.rate_limited_until !== null && account.rate_limited_until > nowUnix) {
      banners.push({ kind: "rate_limited", account, until: account.rate_limited_until });
    }
  }
  return banners;
}

const RUN_RANK: Record<RunState, number> = {
  failed: 0,
  running: 1,
  queued: 1,
  success: 2,
  canceled: 3,
  neutral: 3,
};

/** Orders a repo's workflow runs worst first, then most recently updated first. */
export function sortRuns(runs: Run[]): Run[] {
  return [...runs].sort(
    (a, b) => RUN_RANK[a.state] - RUN_RANK[b.state] || b.updated_at.localeCompare(a.updated_at),
  );
}

/** Dot color for a single run. */
export function runColor(state: RunState): TrayColor {
  switch (state) {
    case "failed":
      return "red";
    case "running":
    case "queued":
      return "yellow";
    case "success":
      return "green";
    default:
      return "gray";
  }
}

export interface Summary {
  headline: string;
  detail: string;
}

/** Header text: the most urgent count as the headline, the other non-zero counts as detail. */
export function summarize(snapshot: Snapshot): Summary {
  const count = (status: RepoStatus) => snapshot.repos.filter((r) => r.state.status === status).length;
  const errors = count("error");
  const parts: [number, string][] = [
    [count("failed"), "failed"],
    [errors, errors === 1 ? "error" : "errors"],
    [count("running"), "running"],
    [count("success"), "passing"],
    [count("none"), "no runs"],
  ];
  const present = parts.filter(([n]) => n > 0).map(([n, label]) => `${n} ${label}`);
  if (snapshot.paused) {
    return { headline: "Paused", detail: present.join(" · ") };
  }
  if (snapshot.repos.length === 0) {
    return { headline: "No repositories", detail: "" };
  }
  const urgent = parts.slice(0, 3).findIndex(([n]) => n > 0);
  if (urgent === -1) {
    return { headline: "All passing", detail: present.join(" · ") };
  }
  const [headline, ...rest] = present;
  return { headline, detail: rest.join(" · ") };
}

/** Spoken and tooltip name of a repo's overall state. */
export function statusLabel(status: RepoStatus): string {
  switch (status) {
    case "failed":
      return "Failed";
    case "error":
      return "Error";
    case "running":
      return "Running";
    case "success":
      return "Passing";
    default:
      return "No runs";
  }
}

/** Spoken and tooltip name of a single run's state. */
export function runStateLabel(state: RunState): string {
  switch (state) {
    case "failed":
      return "Failed";
    case "running":
      return "Running";
    case "queued":
      return "Queued";
    case "success":
      return "Passing";
    case "canceled":
      return "Canceled";
    default:
      return "No result";
  }
}

/** Polling interval for the footer: seconds under a minute, otherwise whole minutes. */
export function formatInterval(seconds: number): string {
  if (seconds < 60) {
    return `${Math.round(seconds)} s`;
  }
  return `${Math.round(seconds / 60)} min`;
}

/** Clock time such as "14:05", in the user's locale. */
function clockTime(unix: number): string {
  return new Date(unix * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** User-facing text of a banner that is plain text. The Keychain and rejected-token banners add their own button. */
export function bannerMessage(banner: Banner): string {
  switch (banner.kind) {
    case "secrets_blocked":
      return "Vigia can’t read its tokens from the Keychain.";
    case "config_error":
      return "Vigia couldn’t read its settings file, so changes won’t be saved.";
    case "config_read_only":
      return "Settings were saved by a newer version of Vigia. Update Vigia to change them.";
    case "auth":
      return authPopupMessage(authReason(banner.account), banner.account.label);
    case "unreachable":
      return `${banner.account.label} is unreachable. Showing the last known state.`;
    case "rate_limited": {
      const provider = banner.account.kind === "gitlab" ? "GitLab" : "GitHub";
      return `${banner.account.label}: paused until ${clockTime(banner.until)} to stay within the ${provider} API limit.`;
    }
    default:
      return "";
  }
}
