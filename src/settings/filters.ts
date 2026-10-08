import type { Organization, OrgFilters, WatchedRepo } from "../lib/tauri";

/** Splits a comma or newline separated list of globs, dropping blanks. */
export function parsePatterns(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map((p) => p.trim())
    .filter((p) => p !== "");
}

/** Whether two pattern lists hold the same globs in the same order. */
export function samePatterns(a: string[], b: string[]): boolean {
  const left = a.join("\n");
  const right = b.join("\n");
  return left === right;
}

export type FilterMode = "default" | "custom";

/** Whether a pattern list inherits (null or absent) or is set explicitly, even when empty. */
export function modeOf(patterns: string[] | null | undefined): FilterMode {
  return patterns == null ? "default" : "custom";
}

/** The pattern list a mode saves when it is chosen: null inherits, an empty list means "nothing". */
export function patternsOfMode(mode: FilterMode): string[] | null {
  return mode === "custom" ? [] : null;
}

export const MODE_OPTIONS: { value: FilterMode; label: string }[] = [
  { value: "default", label: "Default" },
  { value: "custom", label: "Custom" },
];

export type TagChoice = "inherit" | "on" | "off";

/** The pop-up choice for a tag override: null inherits. */
export function tagChoiceOf(include: boolean | null | undefined): TagChoice {
  switch (include) {
    case true:
      return "on";
    case false:
      return "off";
    default:
      return "inherit";
  }
}

/** The override a pop-up choice saves. */
export function tagOverrideOf(choice: TagChoice): boolean | null {
  switch (choice) {
    case "on":
      return true;
    case "off":
      return false;
    default:
      return null;
  }
}

/** The pop-up options for a tag override; the first names the inherited setting it follows. */
export function tagOptions(inheritedOn: boolean): { value: TagChoice; label: string }[] {
  return [
    { value: "inherit", label: inheritedOn ? "Default (On)" : "Default (Off)" },
    { value: "on", label: "On" },
    { value: "off", label: "Off" },
  ];
}

/** A pattern list as text, or `empty` when it holds no patterns. */
export function patternsText(patterns: string[], empty: string): string {
  return patterns.length > 0 ? patterns.join(", ") : empty;
}

/**
 * The placeholder of a pattern field: the inherited list while the mode is default, and what an
 * empty custom list means (`empty`) otherwise.
 */
export function patternPlaceholder(mode: FilterMode, inherited: string[], empty: string): string {
  return mode === "custom" ? empty : patternsText(inherited, empty);
}

/** A stable key for a watched repository. */
export function repoKey(repo: WatchedRepo): string {
  return `${repo.account_id}-${repo.repo.id}`;
}

/** Whether a repository or organization sets any of the three filters. */
export function hasOverrides(filters: Partial<OrgFilters>): boolean {
  return filters.branch_patterns != null || filters.ignored_workflows != null || filters.include_tags != null;
}

/** The values a scope falls back to when one of its filters is Default. */
export interface InheritedFilters {
  branch_patterns: string[];
  ignored_workflows: string[];
  include_tags: boolean;
}

/** The three filters of a repository or organization, with absent values read as Default. */
export function filtersOf(source: Partial<OrgFilters>): OrgFilters {
  return {
    branch_patterns: source.branch_patterns ?? null,
    ignored_workflows: source.ignored_workflows ?? null,
    include_tags: source.include_tags ?? null,
  };
}

/** The values a scope's Default resolves to: its own setting where set, otherwise `fallback`. */
export function resolveFilters(filters: OrgFilters, fallback: InheritedFilters): InheritedFilters {
  return {
    branch_patterns: filters.branch_patterns ?? fallback.branch_patterns,
    ignored_workflows: filters.ignored_workflows ?? fallback.ignored_workflows,
    include_tags: filters.include_tags ?? fallback.include_tags,
  };
}

/** A stable key for an organization: its host and owner. */
export function orgKey(org: Pick<Organization, "host" | "owner">): string {
  return `${org.host}/${org.owner}`;
}

/** Whether a watched repository belongs to an organization. */
export function inOrg(repo: WatchedRepo, org: Organization): boolean {
  return repo.host === org.host && repo.owner === org.owner;
}

/** A repository's name without its owner, such as "widgets" for "acme/widgets". */
export function repoName(repo: WatchedRepo): string {
  const prefix = `${repo.owner}/`;
  const fullName = repo.repo.full_name;
  return fullName.startsWith(prefix) ? fullName.slice(prefix.length) : fullName;
}

/** What a repository's overrides set, such as "Branches: release/* · Tag runs: On". */
export function exceptionSummary(repo: WatchedRepo): string {
  const parts: string[] = [];
  if (repo.branch_patterns != null) {
    parts.push(`Branches: ${patternsText(repo.branch_patterns, "Default branch")}`);
  }
  if (repo.ignored_workflows != null) {
    parts.push(`Ignored workflows: ${patternsText(repo.ignored_workflows, "None")}`);
  }
  if (repo.include_tags != null) {
    parts.push(`Tag runs: ${repo.include_tags ? "On" : "Off"}`);
  }
  return parts.join(" · ");
}
