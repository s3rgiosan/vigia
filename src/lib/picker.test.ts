import { describe, expect, it } from "vitest";
import { buildPickerGroups, flattenGroups, toggleGroup } from "./picker";
import type { PickerRepo } from "./tauri";

function item(id: number, fullName: string, via: string | null = null): PickerRepo {
  return {
    repo: { id, full_name: fullName, web_url: `https://github.com/${fullName}`, default_branch: "main" },
    watched: false,
    watched_via: via,
  };
}

const repos = [item(1, "acme/b"), item(2, "acme/a"), item(3, "other/x"), item(4, "acme/taken", "Home")];

describe("buildPickerGroups", () => {
  it("groups by org, sorts, and computes the tri-state", () => {
    const groups = buildPickerGroups(repos, "", new Set([2]));
    expect(groups.map((g) => g.org)).toEqual(["acme", "other"]);
    expect(groups[0].repos.map((r) => r.repo.full_name)).toEqual(["acme/a", "acme/b", "acme/taken"]);
    expect(groups[0].state).toBe("some");
    expect(groups[1].state).toBe("none");
    expect(buildPickerGroups(repos, "", new Set([1, 2]))[0].state).toBe("all");
  });

  it("sorts digit runs as numbers and ignores case", () => {
    const names = [item(1, "acme/svc-10"), item(2, "acme/svc-2"), item(3, "acme/Svc-1"), item(4, "Beta/x"), item(5, "alpha/y")];
    const groups = buildPickerGroups(names, "", new Set());
    expect(groups.map((g) => g.org)).toEqual(["acme", "alpha", "Beta"]);
    expect(groups[0].repos.map((r) => r.repo.full_name)).toEqual(["acme/Svc-1", "acme/svc-2", "acme/svc-10"]);
  });

  it("filters by search", () => {
    const groups = buildPickerGroups(repos, "other", new Set());
    expect(groups.map((g) => g.org)).toEqual(["other"]);
  });
});

describe("toggleGroup", () => {
  it("selects all selectable repos, then clears them", () => {
    const groups = buildPickerGroups(repos, "", new Set());
    const all = toggleGroup(groups[0], new Set());
    expect([...all].sort()).toEqual([1, 2]);
    const cleared = toggleGroup(buildPickerGroups(repos, "", all)[0], all);
    expect(cleared.size).toBe(0);
  });
});

describe("groups where every repo is watched elsewhere", () => {
  const taken = [item(1, "acme/x", "Home"), item(2, "acme/y", "Home"), item(3, "other/z")];

  it("has no selectable repos and reads as none", () => {
    const groups = buildPickerGroups(taken, "", new Set());
    expect(groups[0].org).toBe("acme");
    expect(groups[0].state).toBe("none");
  });

  it("leaves the selection untouched when toggled", () => {
    const groups = buildPickerGroups(taken, "", new Set());
    const next = toggleGroup(groups[0], new Set([3]));
    expect([...next]).toEqual([3]);
  });
});

describe("search combined with a group toggle", () => {
  const many = [item(1, "acme/api"), item(2, "acme/web"), item(3, "acme/api-docs"), item(4, "other/api")];

  it("toggles only the repos the search leaves in the group", () => {
    const groups = buildPickerGroups(many, "api", new Set());
    const acme = groups.find((g) => g.org === "acme")!;
    expect(acme.repos.map((r) => r.repo.id)).toEqual([1, 3]);
    const next = toggleGroup(acme, new Set());
    expect([...next].sort()).toEqual([1, 3]);
  });

  it("reads the state from the visible repos only", () => {
    const groups = buildPickerGroups(many, "api", new Set([1, 3]));
    expect(groups.find((g) => g.org === "acme")!.state).toBe("all");
  });

  it("keeps picks outside the search when clearing the visible ones", () => {
    const selected = new Set([1, 2, 3]);
    const acme = buildPickerGroups(many, "api", selected).find((g) => g.org === "acme")!;
    expect([...toggleGroup(acme, selected)]).toEqual([2]);
  });
});

describe("flattenGroups", () => {
  it("puts each org header before its repos", () => {
    const rows = flattenGroups(buildPickerGroups(repos, "", new Set()));
    expect(rows.map((r) => (r.kind === "org" ? `org:${r.group.org}` : r.item.repo.full_name))).toEqual([
      "org:acme",
      "acme/a",
      "acme/b",
      "acme/taken",
      "org:other",
      "other/x",
    ]);
    expect(rows[0].kind === "org" && rows[0].first).toBe(true);
    expect(rows[4].kind === "org" && rows[4].first).toBe(false);
  });
});
