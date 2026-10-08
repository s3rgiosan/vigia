// Runtime shape checks for payloads that cross the backend boundary.

import type { Snapshot } from "./snapshot";
import type { SettingsView } from "./tauri";

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isArrayOf(value: unknown, check: (item: unknown) => boolean): boolean {
  return Array.isArray(value) && value.every(check);
}

function isRepoInfo(value: unknown): boolean {
  return isObject(value) && typeof value.id === "number" && typeof value.full_name === "string";
}

function isAccountSnapshot(value: unknown): boolean {
  return isObject(value) && typeof value.id === "string" && typeof value.label === "string";
}

function isRepoSnapshot(value: unknown): boolean {
  return (
    isObject(value) &&
    typeof value.account_id === "string" &&
    isRepoInfo(value.repo) &&
    isObject(value.state) &&
    Array.isArray(value.state.groups) &&
    typeof value.state.status === "string"
  );
}

function isUpdateInfo(value: unknown): boolean {
  return isObject(value) && typeof value.version === "string" && (value.notes === null || typeof value.notes === "string");
}

export function isSnapshot(value: unknown): value is Snapshot {
  return (
    isObject(value) &&
    typeof value.generated_at === "string" &&
    typeof value.paused === "boolean" &&
    typeof value.color === "string" &&
    typeof value.tooltip === "string" &&
    isArrayOf(value.accounts, isAccountSnapshot) &&
    isArrayOf(value.repos, isRepoSnapshot) &&
    (value.update === null || isUpdateInfo(value.update))
  );
}

function isStringArrayOrNull(value: unknown): boolean {
  return value === null || isArrayOf(value, (item) => typeof item === "string");
}

function isOrgFilters(value: unknown): boolean {
  return (
    isObject(value) &&
    isStringArrayOrNull(value.branch_patterns) &&
    isStringArrayOrNull(value.ignored_workflows) &&
    (value.include_tags === null || typeof value.include_tags === "boolean")
  );
}

function isOrganization(value: unknown): boolean {
  return (
    isObject(value) &&
    typeof value.host === "string" &&
    typeof value.owner === "string" &&
    typeof value.repo_count === "number" &&
    isOrgFilters(value.filters)
  );
}

function isAccount(value: unknown): boolean {
  return isObject(value) && typeof value.id === "string" && typeof value.label === "string";
}

function isWatchedRepo(value: unknown): boolean {
  return (
    isObject(value) &&
    typeof value.account_id === "string" &&
    typeof value.host === "string" &&
    typeof value.owner === "string" &&
    isRepoInfo(value.repo)
  );
}

export function isSettingsView(value: unknown): value is SettingsView {
  return (
    isObject(value) &&
    isArrayOf(value.accounts, isAccount) &&
    isArrayOf(value.organizations, isOrganization) &&
    isArrayOf(value.repos, isWatchedRepo) &&
    isObject(value.settings) &&
    typeof value.settings.poll_interval_secs === "number" &&
    typeof value.settings.check_for_updates === "boolean" &&
    Array.isArray(value.settings.branch_patterns) &&
    typeof value.read_only === "boolean" &&
    typeof value.secrets_blocked === "boolean"
  );
}
