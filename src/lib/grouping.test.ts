import { describe, expect, it } from "vitest";
import {
  type Banner,
  bannerMessage,
  buildBanners,
  buildSections,
  formatInterval,
  matchesFilter,
  orgOf,
  relativeTime,
  runStateLabel,
  statusLabel,
  summarize,
} from "./grouping";
import { makeAccount, makeRepo, makeRun, makeSnapshot } from "../test/fixtures";
import type { AccountSnapshot, RepoSnapshot, RepoStatus, Run, Snapshot } from "./snapshot";

function account(id: string, label: string, extra: Partial<AccountSnapshot> = {}): AccountSnapshot {
  return makeAccount({ id, label, ...extra });
}

function run(branch: string, name: string): Run {
  return makeRun({ branch, name, group: "ci", updated_at: "2026-06-01T11:00:00Z" });
}

function repo(accountId: string, fullName: string, status: RepoStatus, rep: Run | null = null): RepoSnapshot {
  const built = makeRepo(fullName, status, rep ? [rep] : [], {}, accountId);
  return { ...built, repo: { ...built.repo, id: fullName.length } };
}

function snapshot(accounts: AccountSnapshot[], repos: RepoSnapshot[]): Snapshot {
  return makeSnapshot({ generated_at: "2026-06-01T12:00:00Z", accounts, repos });
}

describe("orgOf", () => {
  it("takes everything before the last slash", () => {
    expect(orgOf("acme/widgets")).toBe("acme");
    expect(orgOf("group/sub/project")).toBe("group/sub");
    expect(orgOf("solo")).toBe("solo");
  });
});

describe("buildSections", () => {
  it("places repos in sections in order and groups by account then org", () => {
    const a = account("a", "Work");
    const b = account("b", "Home");
    const s = snapshot(
      [a, b],
      [
        repo("a", "acme/z", "failed", run("main", "CI")),
        repo("a", "acme/a", "failed", run("main", "CI")),
        repo("b", "me/x", "failed"),
        repo("a", "acme/ok", "success"),
        repo("a", "acme/err", "error"),
        repo("a", "acme/run", "running"),
        repo("a", "acme/empty", "none"),
      ],
    );
    const sections = buildSections(s, "");
    expect(sections.map((x) => x.id)).toEqual(["failed", "error", "running", "passing", "none"]);
    expect(sections.map((x) => x.count)).toEqual([3, 1, 1, 1, 1]);
    expect(sections.map((x) => x.open)).toEqual([true, true, true, false, false]);

    const failed = sections[0];
    expect(failed.groups.map((g) => `${g.account.label}/${g.org}`)).toEqual(["Home/me", "Work/acme"]);
    expect(failed.groups[1].repos.map((r) => r.repo.full_name)).toEqual(["acme/a", "acme/z"]);
  });

  it("applies the filter to name, branch and run name", () => {
    const a = account("a", "Work");
    const s = snapshot(
      [a],
      [repo("a", "acme/widgets", "failed", run("release/2", "Deploy")), repo("a", "acme/other", "failed", run("main", "CI"))],
    );
    expect(buildSections(s, "widg")[0].count).toBe(1);
    expect(buildSections(s, "release")[0].count).toBe(1);
    expect(buildSections(s, "deploy")[0].count).toBe(1);
    expect(buildSections(s, "nothing")[0].count).toBe(0);
  });

  it("tolerates a repo whose account is missing", () => {
    const s = snapshot([], [repo("ghost", "x/y", "none")]);
    const none = buildSections(s, "")[4];
    expect(none.groups[0].account.label).toBe("Unknown account");
  });
});

describe("matchesFilter", () => {
  it("matches everything on an empty filter", () => {
    expect(matchesFilter(repo("a", "x/y", "none"), "   ")).toBe(true);
  });
});

describe("relativeTime", () => {
  const now = new Date("2026-06-01T12:00:00Z");
  it("formats seconds, minutes, hours and days", () => {
    expect(relativeTime("2026-06-01T11:59:40Z", now)).toBe("just now");
    expect(relativeTime("2026-06-01T11:45:00Z", now)).toBe("15m ago");
    expect(relativeTime("2026-06-01T09:00:00Z", now)).toBe("3h ago");
    expect(relativeTime("2026-05-25T12:00:00Z", now)).toBe("7d ago");
  });
});

describe("buildBanners", () => {
  it("emits one banner per affected account, auth first", () => {
    const s = snapshot(
      [
        account("a", "Auth", { auth_error: true, unreachable: true }),
        account("b", "Down", { unreachable: true }),
        account("c", "Limited", { rate_limited_until: 2000 }),
        account("d", "Expired", { rate_limited_until: 500 }),
        account("e", "Fine"),
      ],
      [],
    );
    const banners = buildBanners(s, 1000);
    expect(banners.map((b) => ("account" in b ? `${b.kind}:${b.account.label}` : b.kind))).toEqual([
      "auth:Auth",
      "unreachable:Down",
      "rate_limited:Limited",
    ]);
  });

  it("words an auth banner by the account's reason", () => {
    const s = snapshot([account("a", "Auth", { auth_error: true, auth_reason: "missing_token" })], []);
    expect(bannerMessage(buildBanners(s, 0)[0])).toBe("No token is saved for “Auth”.");
  });

  it("leaves a Keychain-caused auth error to the Keychain banner while it is shown", () => {
    const a = account("a", "Auth", { auth_error: true, auth_reason: "keychain" });
    expect(buildBanners({ ...snapshot([a], []), secrets_blocked: true }, 0).map((b) => b.kind)).toEqual(["secrets_blocked"]);
    expect(buildBanners(snapshot([a], []), 0).map((b) => b.kind)).toEqual(["auth"]);
  });

  it("puts app-level banners first", () => {
    const s = { ...snapshot([account("a", "Auth", { auth_error: true })], []), secrets_blocked: true, config_read_only: true };
    expect(buildBanners(s, 0).map((b) => b.kind)).toEqual(["secrets_blocked", "config_read_only", "auth"]);
  });

  it("shows only the load error when an unreadable config also makes the store read-only", () => {
    const s = { ...snapshot([], []), config_read_only: true, config_error: "disk error" };
    expect(buildBanners(s, 0).map((b) => b.kind)).toEqual(["config_error"]);
  });
});

describe("sortRuns", () => {
  it("orders worst first, then most recently updated", async () => {
    const { sortRuns } = await import("./grouping");
    const at = (id: number, state: Run["state"], updated: string): Run => ({
      ...run("main", `wf${id}`),
      id,
      state,
      updated_at: updated,
    });
    const sorted = sortRuns([
      at(1, "success", "2026-06-01T11:59:00Z"),
      at(2, "failed", "2026-06-01T10:00:00Z"),
      at(3, "running", "2026-06-01T11:00:00Z"),
      at(4, "failed", "2026-06-01T11:30:00Z"),
      at(5, "queued", "2026-06-01T11:10:00Z"),
    ]);
    expect(sorted.map((r) => r.id)).toEqual([4, 2, 5, 3, 1]);
  });
});

describe("runColor", () => {
  it("maps run states to dot colors", async () => {
    const { runColor } = await import("./grouping");
    expect(runColor("failed")).toBe("red");
    expect(runColor("running")).toBe("yellow");
    expect(runColor("queued")).toBe("yellow");
    expect(runColor("success")).toBe("green");
    expect(runColor("canceled")).toBe("gray");
  });
});

describe("summarize", () => {
  it("lists non-zero counts worst first and falls back to all passing", async () => {
    const { summarize } = await import("./grouping");
    const a = account("a", "Work");
    const s = snapshot(
      [a],
      [repo("a", "x/a", "failed"), repo("a", "x/b", "failed"), repo("a", "x/c", "running"), repo("a", "x/d", "success")],
    );
    expect(summarize(s)).toEqual({ headline: "2 failed", detail: "1 running · 1 passing" });
    const ok = snapshot([a], [repo("a", "x/d", "success"), repo("a", "x/e", "none")]);
    expect(summarize(ok)).toEqual({ headline: "All passing", detail: "1 passing · 1 no runs" });
    expect(summarize({ ...ok, paused: true }).headline).toBe("Paused");
    expect(summarize(snapshot([a], [])).headline).toBe("No repositories");
  });
});

describe("relativeTime edge cases", () => {
  const now = new Date("2026-06-01T12:00:00Z");

  it("returns an empty string for an unparseable timestamp", () => {
    expect(relativeTime("not a date", now)).toBe("");
    expect(relativeTime("", now)).toBe("");
  });

  it("switches from minutes to hours at 60 minutes", () => {
    expect(relativeTime("2026-06-01T11:01:00Z", now)).toBe("59m ago");
    expect(relativeTime("2026-06-01T11:00:00Z", now)).toBe("1h ago");
  });

  it("switches from hours to days at 48 hours", () => {
    expect(relativeTime("2026-05-30T13:00:00Z", now)).toBe("47h ago");
    expect(relativeTime("2026-05-30T12:00:00Z", now)).toBe("2d ago");
  });

  it("treats a future timestamp as just now", () => {
    expect(relativeTime("2026-06-01T12:05:00Z", now)).toBe("just now");
  });
});

describe("summarize", () => {
  it("names the paused state and keeps the counts as detail", () => {
    const s = snapshot([account("a", "Work")], [repo("a", "acme/a", "failed"), repo("a", "acme/b", "success")]);
    expect(summarize({ ...s, paused: true })).toEqual({ headline: "Paused", detail: "1 failed · 1 passing" });
  });

  it("reports paused with no repositories", () => {
    expect(summarize({ ...snapshot([], []), paused: true })).toEqual({ headline: "Paused", detail: "" });
  });

  it("reports no repositories when none are watched", () => {
    expect(summarize(snapshot([account("a", "Work")], []))).toEqual({ headline: "No repositories", detail: "" });
  });
});

describe("buildBanners config error", () => {
  it("includes the message", () => {
    const s = { ...snapshot([], []), config_error: "bad syntax" };
    expect(buildBanners(s, 0)).toEqual([{ kind: "config_error", message: "bad syntax" }]);
  });
});

describe("section titles", () => {
  it("uses one word set for states", () => {
    const s = snapshot([account("a", "Work")], []);
    expect(buildSections(s, "").map((x) => x.title)).toEqual(["Failed", "Errors", "Running", "Passing", "No runs"]);
  });
});

describe("summarize wording", () => {
  it("pluralizes errors", () => {
    const a = account("a", "Work");
    expect(summarize(snapshot([a], [repo("a", "x/a", "error")])).headline).toBe("1 error");
    expect(summarize(snapshot([a], [repo("a", "x/a", "error"), repo("a", "x/b", "error")])).headline).toBe("2 errors");
  });
});

describe("state labels", () => {
  it("names every repo status", () => {
    expect(statusLabel("failed")).toBe("Failed");
    expect(statusLabel("error")).toBe("Error");
    expect(statusLabel("running")).toBe("Running");
    expect(statusLabel("success")).toBe("Passing");
    expect(statusLabel("none")).toBe("No runs");
  });

  it("names every run state", () => {
    expect(runStateLabel("failed")).toBe("Failed");
    expect(runStateLabel("running")).toBe("Running");
    expect(runStateLabel("queued")).toBe("Queued");
    expect(runStateLabel("success")).toBe("Passing");
    expect(runStateLabel("canceled")).toBe("Canceled");
    expect(runStateLabel("neutral")).toBe("No result");
  });
});

describe("formatInterval", () => {
  it("uses seconds under a minute and whole minutes after", () => {
    expect(formatInterval(45)).toBe("45 s");
    expect(formatInterval(144)).toBe("2 min");
    expect(formatInterval(600)).toBe("10 min");
  });
});

describe("bannerMessage", () => {
  it("words the app-level banners", () => {
    expect(bannerMessage({ kind: "config_error", message: "bad" })).toBe(
      "Vigia couldn’t read its settings file, so changes won’t be saved.",
    );
    expect(bannerMessage({ kind: "config_read_only" })).toBe(
      "Settings were saved by a newer version of Vigia. Update Vigia to change them.",
    );
  });

  it("names the account and provider in the rate-limit banner", () => {
    const text = bannerMessage({ kind: "rate_limited", account: account("a", "Work"), until: 1000 });
    expect(text).toMatch(/^Work: paused until .+ to stay within the GitHub API limit\.$/);
    const lab = bannerMessage({ kind: "rate_limited", account: account("b", "Lab", { kind: "gitlab" }), until: 1000 });
    expect(lab).toContain("GitLab API limit");
  });

  it("words the keychain banner and unreachable accounts", () => {
    expect(bannerMessage({ kind: "secrets_blocked" })).toBe("Vigia can’t read its tokens from the Keychain.");
    expect(bannerMessage({ kind: "unreachable", account: account("a", "Work") })).toBe(
      "Work is unreachable. Showing the last known state.",
    );
  });

  it("returns no text for an unknown banner", () => {
    expect(bannerMessage({ kind: "other" } as unknown as Banner)).toBe("");
  });

  it("words account banners", () => {
    expect(bannerMessage({ kind: "auth", account: account("a", "Work") })).toBe(
      "The token for “Work” was rejected.",
    );
  });
});

describe("matchesFilter without a representative run", () => {
  it("matches on the repository name alone", () => {
    const bare = repo("a", "acme/widgets", "none");
    expect(matchesFilter(bare, "widg")).toBe(true);
    expect(matchesFilter(bare, "main")).toBe(false);
  });
});

describe("buildSections ordering", () => {
  it("orders organisations of one account alphabetically", () => {
    const s = snapshot(
      [account("a", "Work")],
      [repo("a", "zeta/one", "failed"), repo("a", "acme/two", "failed")],
    );
    const orgs = buildSections(s, "")[0].groups.map((g) => g.org);
    expect(orgs).toEqual(["acme", "zeta"]);
  });

  it("sorts repository names by number, ignoring case", () => {
    const s = snapshot(
      [account("a", "Work")],
      [repo("a", "acme/repo-10", "failed"), repo("a", "acme/Repo-2", "failed"), repo("a", "acme/repo-1", "failed")],
    );
    const names = buildSections(s, "")[0].groups[0].repos.map((r) => r.repo.full_name);
    expect(names).toEqual(["acme/repo-1", "acme/Repo-2", "acme/repo-10"]);
  });
});
