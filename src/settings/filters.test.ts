import { describe, expect, it } from "vitest";
import { makeOrganization, makeWatchedRepo } from "../test/fixtures";
import {
  exceptionSummary,
  filtersOf,
  hasOverrides,
  inOrg,
  modeOf,
  orgKey,
  patternPlaceholder,
  patternsOfMode,
  parsePatterns,
  patternsText,
  repoKey,
  repoName,
  resolveFilters,
  samePatterns,
  tagChoiceOf,
  tagOptions,
  tagOverrideOf,
} from "./filters";

describe("parsePatterns", () => {
  it("splits on commas and newlines and trims", () => {
    expect(parsePatterns("main, release/*\n hotfix/* ")).toEqual(["main", "release/*", "hotfix/*"]);
  });

  it("drops empty entries", () => {
    expect(parsePatterns(",, ,\n\n")).toEqual([]);
    expect(parsePatterns("")).toEqual([]);
  });

  it("keeps duplicates in order", () => {
    expect(parsePatterns("main,main")).toEqual(["main", "main"]);
  });
});

describe("modeOf and patternsOfMode", () => {
  it("treats null and absent lists as default and any list, even empty, as custom", () => {
    expect(modeOf(null)).toBe("default");
    expect(modeOf(undefined)).toBe("default");
    expect(modeOf([])).toBe("custom");
    expect(modeOf(["main"])).toBe("custom");
  });

  it("saves null for default and an empty list for custom", () => {
    expect(patternsOfMode("default")).toBeNull();
    expect(patternsOfMode("custom")).toEqual([]);
  });
});

describe("patternPlaceholder", () => {
  it("shows the inherited list in default mode and what empty means in custom mode", () => {
    expect(patternPlaceholder("default", ["main", "dev"], "Default branch")).toBe("main, dev");
    expect(patternPlaceholder("default", [], "Default branch")).toBe("Default branch");
    expect(patternPlaceholder("custom", ["main"], "None")).toBe("None");
  });
});

describe("parsePatterns for workflow globs", () => {
  it("keeps spaces inside a name", () => {
    expect(parsePatterns("Dependabot Updates, nightly*")).toEqual(["Dependabot Updates", "nightly*"]);
  });
});

describe("samePatterns", () => {
  it("compares order and content", () => {
    expect(samePatterns(["a", "b"], ["a", "b"])).toBe(true);
    expect(samePatterns(["a", "b"], ["b", "a"])).toBe(false);
  });
});

describe("tag overrides", () => {
  it("maps pop-up choices to overrides and back", () => {
    expect(tagChoiceOf(true)).toBe("on");
    expect(tagChoiceOf(false)).toBe("off");
    expect(tagChoiceOf(null)).toBe("inherit");
    expect(tagChoiceOf(undefined)).toBe("inherit");
    expect(tagOverrideOf("on")).toBe(true);
    expect(tagOverrideOf("off")).toBe(false);
    expect(tagOverrideOf("inherit")).toBeNull();
  });

  it("names the global setting in the default option", () => {
    expect(tagOptions(true).map((o) => o.label)).toEqual(["Default (On)", "On", "Off"]);
    expect(tagOptions(false)[0].label).toBe("Default (Off)");
  });
});

describe("repo helpers", () => {
  it("formats pattern lists", () => {
    expect(patternsText([], "None")).toBe("None");
    expect(patternsText(["main", "release/*"], "None")).toBe("main, release/*");
  });

  it("keys repos by account and id and detects overrides", () => {
    const repo = makeWatchedRepo("acme/widgets", { account_id: "acc-2" });
    expect(repoKey(repo)).toBe(`acc-2-${repo.repo.id}`);
    expect(hasOverrides(repo)).toBe(false);
    expect(hasOverrides({ ...repo, include_tags: false })).toBe(true);
    expect(hasOverrides({ ...repo, ignored_workflows: [] })).toBe(true);
    expect(hasOverrides({ ...repo, branch_patterns: ["main"] })).toBe(true);
  });

  it("names a repo without its owner, keeping names outside the owner whole", () => {
    expect(repoName(makeWatchedRepo("acme/widgets"))).toBe("widgets");
    expect(repoName(makeWatchedRepo("acme/tools/cli"))).toBe("cli");
    expect(repoName(makeWatchedRepo("other/widgets", { owner: "acme" }))).toBe("other/widgets");
  });

  it("summarizes what an exception sets, in row order", () => {
    const repo = makeWatchedRepo("acme/widgets");
    expect(exceptionSummary(repo)).toBe("");
    expect(exceptionSummary({ ...repo, branch_patterns: ["release/*"], ignored_workflows: [], include_tags: true })).toBe(
      "Branches: release/* · Ignored workflows: None · Tag runs: On",
    );
    expect(exceptionSummary({ ...repo, branch_patterns: [], ignored_workflows: ["nightly"], include_tags: false })).toBe(
      "Branches: Default branch · Ignored workflows: nightly · Tag runs: Off",
    );
  });
});

describe("organization helpers", () => {
  it("keys organizations by host and owner and matches their repos", () => {
    const org = makeOrganization("acme");
    expect(orgKey(org)).toBe("github.com/acme");
    expect(inOrg(makeWatchedRepo("acme/widgets"), org)).toBe(true);
    expect(inOrg(makeWatchedRepo("acme/widgets", { host: "gitlab.example.com" }), org)).toBe(false);
    expect(inOrg(makeWatchedRepo("other/widgets"), org)).toBe(false);
  });

  it("reads absent filters as Default and resolves Default to the fallback", () => {
    const filters = filtersOf({ ignored_workflows: [] });
    expect(filters).toEqual({ branch_patterns: null, ignored_workflows: [], include_tags: null });
    expect(resolveFilters(filters, { branch_patterns: ["main"], ignored_workflows: ["Dependabot*"], include_tags: true })).toEqual({
      branch_patterns: ["main"],
      ignored_workflows: [],
      include_tags: true,
    });
  });
});
