import type { Snapshot } from "../lib/snapshot";

/**
 * Returns `next` with every repository that is unchanged since `prev` replaced by the previous
 * object, so memoized rows skip rendering for repositories that did not change.
 */
export function shareUnchangedRepos(prev: Snapshot | null, next: Snapshot): Snapshot {
  if (!prev) {
    return next;
  }
  const keyOf = (r: Snapshot["repos"][number]) => `${r.account_id}/${r.repo.id}`;
  const before = new Map(prev.repos.map((r) => [keyOf(r), r]));
  const repos = next.repos.map((r) => {
    const old = before.get(keyOf(r));
    return old && JSON.stringify(old) === JSON.stringify(r) ? old : r;
  });
  return { ...next, repos };
}
