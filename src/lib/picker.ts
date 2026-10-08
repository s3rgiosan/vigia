import { orgOf } from "./grouping";
import type { PickerRepo } from "./tauri";

export interface PickerGroup {
  org: string;
  repos: PickerRepo[];
  /** How many selectable repos in this group are selected: none, some, or all. */
  state: "none" | "some" | "all";
}

/** Orders names the way Finder does: case-insensitive, with digit runs compared as numbers. */
function compareNames(a: string, b: string): number {
  return a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" });
}

/** Groups picker repos by org, applies the search, and computes each group's checkbox state. */
export function buildPickerGroups(repos: PickerRepo[], search: string, selected: Set<number>): PickerGroup[] {
  const needle = search.trim().toLowerCase();
  const byOrg = new Map<string, PickerRepo[]>();
  for (const item of repos) {
    if (needle !== "" && !item.repo.full_name.toLowerCase().includes(needle)) {
      continue;
    }
    const org = orgOf(item.repo.full_name);
    const list = byOrg.get(org) ?? [];
    list.push(item);
    byOrg.set(org, list);
  }
  return [...byOrg.entries()]
    .sort(([a], [b]) => compareNames(a, b))
    .map(([org, list]) => {
      list.sort((a, b) => compareNames(a.repo.full_name, b.repo.full_name));
      const selectable = list.filter((r) => r.watched_via === null);
      const picked = selectable.filter((r) => selected.has(r.repo.id)).length;
      const state = picked === 0 ? "none" : picked === selectable.length ? "all" : "some";
      return { org, repos: list, state };
    });
}

/** Toggles every selectable repo of a group: all on unless they already are, then all off. */
export function toggleGroup(group: PickerGroup, selected: Set<number>): Set<number> {
  const next = new Set(selected);
  const selectable = group.repos.filter((r) => r.watched_via === null);
  if (group.state === "all") {
    for (const r of selectable) {
      next.delete(r.repo.id);
    }
  } else {
    for (const r of selectable) {
      next.add(r.repo.id);
    }
  }
  return next;
}

export type PickerRow =
  | { kind: "org"; group: PickerGroup; first: boolean }
  | { kind: "repo"; item: PickerRepo };

/** Flattens groups into one list of org header rows and repo rows, so they can be windowed. */
export function flattenGroups(groups: PickerGroup[]): PickerRow[] {
  const rows: PickerRow[] = [];
  groups.forEach((group, index) => {
    rows.push({ kind: "org", group, first: index === 0 });
    for (const item of group.repos) {
      rows.push({ kind: "repo", item });
    }
  });
  return rows;
}
