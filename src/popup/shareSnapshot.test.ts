import { describe, expect, it } from "vitest";
import { makeRepo, makeRun, makeSnapshot } from "../test/fixtures";
import { shareUnchangedRepos } from "./shareSnapshot";

describe("shareUnchangedRepos", () => {
  const build = () =>
    makeSnapshot({
      repos: [makeRepo("acme/api", "failed", [makeRun()]), makeRepo("acme/webapp", "success", [makeRun({ state: "success" })])],
    });

  it("returns the new snapshot when there is no previous one", () => {
    const next = build();
    expect(shareUnchangedRepos(null, next)).toBe(next);
  });

  it("reuses unchanged repositories and takes changed or new ones from the new snapshot", () => {
    const prev = build();
    const next = build();
    next.repos[1] = makeRepo("acme/webapp", "failed", [makeRun()]);
    next.repos.push(makeRepo("acme/new", "none"));
    const shared = shareUnchangedRepos(prev, next);
    expect(shared.repos[0]).toBe(prev.repos[0]);
    expect(shared.repos[1]).toBe(next.repos[1]);
    expect(shared.repos[2]).toBe(next.repos[2]);
  });
});
