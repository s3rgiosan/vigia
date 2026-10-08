import { describe, expect, it } from "vitest";
import {
  makeAccount,
  makeOrganization,
  makeRepo,
  makeRun,
  makeSettings,
  makeSettingsAccount,
  makeSnapshot,
  makeView,
  makeWatchedRepo,
} from "../test/fixtures";
import { isSettingsView, isSnapshot } from "./guards";

describe("isSnapshot", () => {
  it("accepts a well-formed snapshot", () => {
    const snapshot = makeSnapshot({
      accounts: [makeAccount()],
      repos: [makeRepo("acme/api", "failed", [makeRun()])],
    });
    expect(isSnapshot(snapshot)).toBe(true);
  });

  it("accepts an available update with or without notes", () => {
    expect(isSnapshot(makeSnapshot({ update: { version: "2.0.0", notes: "Fixes." } }))).toBe(true);
    expect(isSnapshot(makeSnapshot({ update: { version: "2.0.0", notes: null } }))).toBe(true);
  });

  it.each([
    ["null", null],
    ["an array", []],
    ["a string", "snapshot"],
    ["a missing field", { ...makeSnapshot(), tooltip: undefined }],
    ["an update without a version", { ...makeSnapshot(), update: { notes: null } }],
    ["an update with numeric notes", { ...makeSnapshot(), update: { version: "2.0.0", notes: 1 } }],
    ["a missing update", { ...makeSnapshot(), update: undefined }],
    ["accounts that are not an array", { ...makeSnapshot(), accounts: {} }],
    ["an account without an id", { ...makeSnapshot(), accounts: [{ label: "x" }] }],
    [
      "a repo without state",
      {
        ...makeSnapshot(),
        repos: [{ account_id: "a", repo: makeRepo("a/b", "none").repo }],
      },
    ],
    [
      "a repo without groups",
      {
        ...makeSnapshot(),
        repos: [
          {
            account_id: "a",
            repo: makeRepo("a/b", "none").repo,
            state: { status: "none" },
          },
        ],
      },
    ],
    [
      "a repo with a bad info",
      {
        ...makeSnapshot(),
        repos: [{ account_id: "a", repo: {}, state: { status: "none", groups: [] } }],
      },
    ],
  ])("rejects %s", (_name, value) => {
    expect(isSnapshot(value)).toBe(false);
  });
});

const NO_FILTERS = { branch_patterns: null, ignored_workflows: null, include_tags: null };

describe("isSettingsView", () => {
  it("accepts organization filters that are null, lists or booleans", () => {
    const filters = { branch_patterns: [], ignored_workflows: ["Dependabot*"], include_tags: false };
    expect(isSettingsView(makeView({ organizations: [makeOrganization("acme", { filters })] }))).toBe(true);
    expect(isSettingsView(makeView({ organizations: [makeOrganization("acme", { filters: NO_FILTERS })] }))).toBe(true);
  });

  it("accepts a well-formed view", () => {
    const view = makeView({
      accounts: [makeSettingsAccount()],
      organizations: [makeOrganization()],
      repos: [makeWatchedRepo()],
    });
    expect(isSettingsView(view)).toBe(true);
  });

  it.each([
    ["undefined", undefined],
    ["a missing settings object", { ...makeView(), settings: null }],
    [
      "settings without a poll interval",
      {
        ...makeView(),
        settings: { ...makeSettings(), poll_interval_secs: "60" },
      },
    ],
    ["settings without branch patterns", { ...makeView(), settings: { ...makeSettings(), branch_patterns: null } }],
    ["settings without the update switch", { ...makeView(), settings: { ...makeSettings(), check_for_updates: undefined } }],
    ["a missing read_only flag", { ...makeView(), read_only: undefined }],
    ["a missing secrets_blocked flag", { ...makeView(), secrets_blocked: 1 }],
    ["an account without an id", { ...makeView(), accounts: [{}] }],
    ["missing organizations", { ...makeView(), organizations: undefined }],
    ["an organization without a host", { ...makeView(), organizations: [{ ...makeOrganization(), host: undefined }] }],
    ["an organization without an owner", { ...makeView(), organizations: [{ ...makeOrganization(), owner: 1 }] }],
    ["an organization without a repo count", { ...makeView(), organizations: [{ ...makeOrganization(), repo_count: "2" }] }],
    ["an organization without filters", { ...makeView(), organizations: [{ ...makeOrganization(), filters: undefined }] }],
    [
      "filters with a non-list branch value",
      { ...makeView(), organizations: [makeOrganization("acme", { filters: { ...NO_FILTERS, branch_patterns: "main" as never } })] },
    ],
    [
      "filters with a non-string workflow",
      { ...makeView(), organizations: [makeOrganization("acme", { filters: { ...NO_FILTERS, ignored_workflows: [1] as never } })] },
    ],
    [
      "filters with a non-boolean tag value",
      { ...makeView(), organizations: [makeOrganization("acme", { filters: { ...NO_FILTERS, include_tags: "yes" as never } })] },
    ],
    ["a watched repo without a repo", { ...makeView(), repos: [{ account_id: "a", host: "github.com", owner: "acme" }] }],
    ["a watched repo without a host", { ...makeView(), repos: [{ ...makeWatchedRepo(), host: undefined }] }],
    ["a watched repo without an owner", { ...makeView(), repos: [{ ...makeWatchedRepo(), owner: null }] }],
  ])("rejects %s", (_name, value) => {
    expect(isSettingsView(value)).toBe(false);
  });
});
