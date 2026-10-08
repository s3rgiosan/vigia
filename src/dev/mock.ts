// Browser preview backend. Loaded only by `npm run dev` outside Tauri, so the windows can be
// designed and checked in a plain browser. All names and data here are fabricated.

import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { NO_ACCESS_NOTE } from "../lib/github";
import type { RepoSnapshot, RepoStatus, Run, RunState, Snapshot, UpdateInfo } from "../lib/snapshot";
import type { OrgFilters, PickerRepo, SettingsView, WatchedRepo } from "../lib/tauri";

const minutesAgo = (m: number) => new Date(Date.now() - m * 60_000).toISOString();

function run(id: number, state: RunState, name: string, branch: string, minutes: number): Run {
  return {
    id,
    attempt: 1,
    state,
    branch,
    group: name,
    name,
    url: `https://github.com/acme/repo/actions/runs/${id}`,
    updated_at: minutesAgo(minutes),
    pull_request: false,
    fork: false,
    tag: false,
  };
}

function repo(accountId: string, fullName: string, status: RepoStatus, runs: Run[], extra: Partial<RepoSnapshot["state"]> = {}): RepoSnapshot {
  return {
    account_id: accountId,
    repo: { id: fullName.length * 7919 + fullName.charCodeAt(0), full_name: fullName, web_url: `https://github.com/${fullName}`, default_branch: "main" },
    state: {
      status,
      representative: runs[0] ?? null,
      groups: runs,
      last_checked: minutesAgo(1),
      stale: false,
      note: null,
      ...extra,
    },
  };
}

const NO_FILTERS: OrgFilters = { branch_patterns: null, ignored_workflows: null, include_tags: null };

const ACCOUNTS: SettingsView["accounts"] = [
  { id: "gh-work", kind: "github", label: "Work", user_id: 1, login: "jdoe" },
  { id: "gh-home", kind: "github", label: "Personal", user_id: 1, login: "jdoe" },
  { id: "gl-corp", kind: "gitlab", label: "Corp GitLab", base_url: "https://gitlab.example.com/", user_id: 9, login: "jdoe" },
];

function hostOf(accountId: string): string {
  return accountId === "gl-corp" ? "gitlab.example.com" : "github.com";
}

const orgKey = (host: string, owner: string) => `${host}/${owner}`;

/** Groups the watched repos by owner, sorted by host, then owner, like the backend. */
function organizations(repos: WatchedRepo[], filters: Map<string, OrgFilters>): SettingsView["organizations"] {
  const counts = new Map<string, { host: string; owner: string; repo_count: number }>();
  for (const r of repos) {
    const key = orgKey(r.host, r.owner);
    const entry = counts.get(key) ?? { host: r.host, owner: r.owner, repo_count: 0 };
    entry.repo_count += 1;
    counts.set(key, entry);
  }
  const compare = (a: string, b: string) => a.localeCompare(b, undefined, { numeric: true });
  return [...counts.entries()]
    .map(([key, entry]) => ({ ...entry, filters: filters.get(key) ?? NO_FILTERS }))
    .sort((a, b) => compare(a.host, b.host) || compare(a.owner, b.owner));
}

const AVAILABLE_UPDATE: UpdateInfo = { version: "1.1.0", notes: "Fabricated release notes." };

function buildSnapshot(paused: boolean, scenario: string): Snapshot {
  const repos: RepoSnapshot[] = [
    repo("gh-work", "acme/billing-api", "failed", [
      run(101, "failed", "CI", "main", 4),
      run(102, "success", "Lint", "main", 4),
      run(103, "running", "Deploy preview", "main", 1),
    ]),
    repo("gh-work", "acme/web-app", "failed", [run(111, "failed", "E2E", "main", 22), run(112, "success", "CI", "main", 22)]),
    repo("gh-work", "acme/mobile", "running", [run(121, "running", "Build", "main", 2), run(122, "success", "Tests", "main", 30)]),
    repo("gh-work", "acme/infra", "error", [], { note: NO_ACCESS_NOTE, last_checked: minutesAgo(3) }),
    repo("gh-work", "acme/design-system", "success", [run(131, "success", "Release", "main", 58)]),
    repo("gh-work", "acme/docs", "success", [run(141, "success", "Pages", "main", 180)]),
    repo("gh-work", "acme/customer-notification-delivery-pipeline-integration-tests", "success", [
      run(145, "success", "CI", "main", 240),
    ]),
    repo("gh-home", "jdoe/dotfiles", "success", [run(151, "success", "Check", "main", 1440)]),
    repo("gh-home", "jdoe/notes", "none", []),
    repo("gl-corp", "platform/gateway", "running", [run(161, "queued", "push", "main", 0)]),
    repo("gl-corp", "platform/auth", "success", [run(171, "success", "push", "main", 95), run(172, "success", "schedule", "main", 600)], { stale: true }),
  ];

  const accounts: Snapshot["accounts"] = ACCOUNTS.map((a) => ({
    id: a.id,
    label: a.label,
    kind: a.kind,
    auth_error: scenario === "banners" && a.id === "gh-home",
    unreachable: scenario === "banners" && a.id === "gl-corp",
    rate_limited_until: null,
    effective_interval_secs: a.id === "gh-work" ? 144 : 60,
    configured_interval_secs: 60,
    keychain_denied: false,
    error: null,
  }));

  const counts = { failed: 0, error: 0, running: 0, success: 0, none: 0 };
  for (const r of repos) counts[r.state.status] += 1;

  return {
    generated_at: minutesAgo(0),
    paused,
    secrets_blocked: false,
    config_read_only: false,
    config_error: scenario === "config_error" ? "expected value at line 3 column 1" : null,
    color: paused ? "gray" : "red",
    tooltip: "",
    accounts: scenario === "empty" ? [] : accounts,
    repos: scenario === "empty" ? [] : repos,
    update: scenario === "update" ? AVAILABLE_UPDATE : null,
  };
}

function pickerRepos(many: boolean): PickerRepo[] {
  const names = many
    ? Array.from({ length: 600 }, (_, i) => `org-${String(Math.floor(i / 40)).padStart(2, "0")}/repo-${i}`)
    : [
    "acme/billing-api",
    "acme/web-app",
    "acme/mobile",
    "acme/infra",
    "acme/design-system",
    "acme/docs",
    "acme/analytics",
    "acme/sandbox",
    "jdoe/dotfiles",
    "jdoe/notes",
      ];
  return names.map((full_name, i) => ({
    repo: { id: i + 1, full_name, web_url: `https://github.com/${full_name}`, default_branch: "main" },
    watched: i < 6,
    watched_via: full_name === "jdoe/notes" ? "Personal" : null,
  }));
}

export function install() {
  Object.defineProperty(window, "__VIGIA_PREVIEW__", { value: true });
  const scenario = new URLSearchParams(window.location.search).get("scenario") ?? "default";
  let paused = false;
  // One organization ignores no workflows, overriding the global Dependabot* pattern.
  const orgFilters = new Map<string, OrgFilters>([[orgKey("github.com", "acme"), { ...NO_FILTERS, ignored_workflows: [] }]]);
  const settings: SettingsView = {
    accounts: scenario === "empty" ? [] : structuredClone(ACCOUNTS),
    organizations: [],
    repos: [],
    settings: {
      poll_interval_secs: 60,
      exclude_pull_requests: true,
      branch_patterns: [],
      ignored_workflows: ["Dependabot*"],
      include_tags: false,
      notify_failures: true,
      notify_recoveries: false,
      launch_at_login: false,
      check_for_updates: true,
    },
    read_only: false,
    secrets_blocked: scenario === "keychain",
  };
  settings.repos = buildSnapshot(false, scenario).repos.map((r) => ({
    account_id: r.account_id,
    repo: r.repo,
    host: hostOf(r.account_id),
    owner: r.repo.full_name.slice(0, r.repo.full_name.lastIndexOf("/")),
  }));
  // One repo starts with overrides so the Filters exceptions list is not empty.
  if (settings.repos.length > 0) {
    settings.repos[0].branch_patterns = ["release/*"];
    settings.repos[0].include_tags = true;
  }

  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "get_snapshot":
          return buildSnapshot(paused, scenario);
        case "set_paused":
          paused = Boolean(a.paused);
          return null;
        case "get_settings":
          // A fresh copy, like the deserialized reply from the real backend.
          return structuredClone({ ...settings, organizations: organizations(settings.repos, orgFilters) });
        case "list_picker_repos":
          return (async () => {
            for (const loaded of [100, 200, 300]) {
              await emit("repo-list-progress", { account_id: a.accountId, loaded });
              await new Promise((resolve) => setTimeout(resolve, 250));
            }
            return pickerRepos(scenario === "many");
          })();
        case "reset_secrets":
          settings.secrets_blocked = false;
          return true;
        case "retry_secrets":
          return !settings.secrets_blocked;
        case "test_connection":
          return { user_id: 1, login: "jdoe" };
        case "unwatch_repo": {
          const index = settings.repos.findIndex(
            (w) => w.account_id === a.accountId && w.repo.id === a.repoId,
          );
          if (index === -1) {
            return null;
          }
          return settings.repos.splice(index, 1)[0];
        }
        case "restore_watched_repo": {
          const watched = a.token as WatchedRepo;
          const present = settings.repos.some(
            (w) => w.account_id === watched.account_id && w.repo.id === watched.repo.id,
          );
          if (present) {
            return false;
          }
          settings.repos.push(watched);
          return true;
        }
        case "set_repo_branches": {
          const entry = settings.repos.find((w) => w.account_id === a.accountId && w.repo.id === a.repoId);
          if (entry) {
            entry.branch_patterns = (a.patterns as string[] | null) ?? null;
          }
          return null;
        }
        case "set_repo_ignored_workflows": {
          const entry = settings.repos.find((w) => w.account_id === a.accountId && w.repo.id === a.repoId);
          if (entry) {
            entry.ignored_workflows = (a.patterns as string[] | null) ?? null;
          }
          return null;
        }
        case "set_repo_include_tags": {
          const entry = settings.repos.find((w) => w.account_id === a.accountId && w.repo.id === a.repoId);
          if (entry) {
            entry.include_tags = (a.include as boolean | null) ?? null;
          }
          return null;
        }
        case "clear_repo_overrides": {
          const entry = settings.repos.find((w) => w.account_id === a.accountId && w.repo.id === a.repoId);
          if (entry) {
            entry.branch_patterns = null;
            entry.ignored_workflows = null;
            entry.include_tags = null;
          }
          return null;
        }
        case "set_org_filters":
          orgFilters.set(orgKey(String(a.host), String(a.owner)), a.filters as OrgFilters);
          return null;
        case "plugin:app|version":
          return "1.0.0";
        case "check_for_updates_now":
          return new Promise((resolve) => setTimeout(() => resolve(scenario === "update" ? AVAILABLE_UPDATE : null), 800));
        case "install_update":
          return (async () => {
            const total = 1_000_000;
            for (let downloaded = 0; downloaded <= total; downloaded += 250_000) {
              await emit("update-progress", { downloaded, total });
              await new Promise((resolve) => setTimeout(resolve, 400));
            }
            return null;
          })();
        case "update_settings":
          settings.settings = a.settings as SettingsView["settings"];
          return null;
        default:
          console.info("[mock] invoke", cmd, args);
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
