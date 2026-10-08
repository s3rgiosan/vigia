import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { FormSection } from "../components/Form";
import { Icon } from "../components/Icon";
import { friendlyError } from "../lib/errors";
import { useRovingListbox } from "../lib/hooks/useRovingListbox";
import {
  clearRepoOverrides,
  setOrgFilters,
  setRepoBranches,
  setRepoIgnoredWorkflows,
  setRepoIncludeTags,
  type Organization,
  type OrgFilters,
  type WatchedRepo,
} from "../lib/tauri";
import { pluralize } from "./accounts";
import { AddExceptionsSheet } from "./AddExceptionsSheet";
import { FilterRows } from "./FilterRows";
import {
  exceptionSummary,
  filtersOf,
  hasOverrides,
  inOrg,
  repoKey,
  repoName,
  resolveFilters,
  type InheritedFilters,
} from "./filters";
import { useSettings } from "./SettingsContext";

/**
 * An organization's own filters and its repository exceptions. `sessionAdded` keeps repositories
 * added through the sheet listed while every filter is still Default.
 */
export function OrgDetail({
  org,
  global,
  sessionAdded,
  onSessionAdd,
  onSessionRemove,
}: {
  org: Organization;
  global: InheritedFilters;
  sessionAdded: string[];
  onSessionAdd: (keys: string[]) => void;
  onSessionRemove: (key: string) => void;
}) {
  const { view, reload, setError, readOnly } = useSettings();
  const [expandedKey, setExpandedKey] = useState<string | null>(null);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const headerRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const repos = view.repos.filter((repo) => inOrg(repo, org));
  const exceptions = repos.filter((repo) => hasOverrides(repo) || sessionAdded.includes(repoKey(repo)));
  const candidates = repos.filter((repo) => !exceptions.includes(repo));
  const accountIds = new Set(repos.map((repo) => repo.account_id));
  const kind = view.accounts.find((a) => accountIds.has(a.id))?.kind;
  const labelOf = (repo: WatchedRepo) => view.accounts.find((a) => a.id === repo.account_id)?.label ?? "";
  const orgResolved = resolveFilters(org.filters, global);
  const selectedIndex = exceptions.findIndex((repo) => repoKey(repo) === selectedKey);
  const selected = selectedIndex >= 0 ? exceptions[selectedIndex] : null;

  const onListKey = useRovingListbox({
    count: exceptions.length,
    activeIndex: selectedIndex,
    onMove: (index) => {
      setSelectedKey(repoKey(exceptions[index]));
      headerRefs.current[index]?.focus();
    },
  });

  // The first repository added through the sheet takes focus once its row shows.
  useEffect(() => {
    if (focusKey) {
      document.getElementById(`exception-${focusKey}`)?.focus();
      setFocusKey(null);
    }
  }, [focusKey]);

  async function run(action: () => Promise<unknown>) {
    try {
      await action();
      setError(null);
      await reload();
    } catch (e) {
      setError(friendlyError(e));
    }
  }

  function saveRepo(repo: WatchedRepo, patch: Partial<OrgFilters>) {
    const { account_id: accountId, repo: info } = repo;
    const { branch_patterns: branches, ignored_workflows: ignored, include_tags: tags } = patch;
    if (branches !== undefined) {
      void run(() => setRepoBranches(accountId, info.id, branches));
    }
    if (ignored !== undefined) {
      void run(() => setRepoIgnoredWorkflows(accountId, info.id, ignored));
    }
    if (tags !== undefined) {
      void run(() => setRepoIncludeTags(accountId, info.id, tags));
    }
  }

  async function removeException(repo: WatchedRepo) {
    const index = exceptions.indexOf(repo);
    const neighbour = exceptions[index + 1] ?? exceptions[index - 1];
    const key = repoKey(repo);
    await run(() => clearRepoOverrides(repo.account_id, repo.repo.id));
    onSessionRemove(key);
    setExpandedKey((current) => (current === key ? null : current));
    setSelectedKey(neighbour ? repoKey(neighbour) : null);
  }

  function addExceptions(keys: string[]) {
    onSessionAdd(keys);
    setExpandedKey(keys[0]);
    setSelectedKey(keys[0]);
    setFocusKey(keys[0]);
    setAddOpen(false);
  }

  function toggle(key: string) {
    setSelectedKey(key);
    setExpandedKey((current) => (current === key ? null : key));
  }

  function onHeaderKey(e: KeyboardEvent<HTMLButtonElement>, key: string) {
    switch (e.key) {
      case "ArrowRight":
        e.preventDefault();
        setExpandedKey(key);
        break;
      case "ArrowLeft":
        e.preventDefault();
        setExpandedKey((current) => (current === key ? null : current));
        break;
      default:
        onListKey(e);
    }
  }

  return (
    <div className="accounts__detail">
      <div className="detail-header">
        <div className="detail-header__icon">
          <Icon name={kind ?? "folder"} size={26} />
        </div>
        <div>
          <h2>{org.owner}</h2>
          <p>
            {org.host} · {pluralize(org.repo_count, "repository", "repositories")} watched
          </p>
        </div>
      </div>

      <FormSection title="Filters" footer="Default uses the settings for all repositories.">
        <FilterRows
          name={org.owner}
          filters={org.filters}
          inherited={global}
          disabled={readOnly}
          onChange={(patch) => run(() => setOrgFilters(org.host, org.owner, { ...org.filters, ...patch }))}
        />
      </FormSection>

      <FormSection title="Repository Exceptions">
        {exceptions.length === 0 ? (
          <div className="exceptions__empty">No exceptions. Every repository in {org.owner} uses the settings above.</div>
        ) : null}
        {exceptions.map((repo, index) => {
          const key = repoKey(repo);
          const name = repo.repo.full_name;
          const expanded = key === expandedKey;
          const isSelected = key === selectedKey;
          const bodyId = `exception-${key}-filters`;
          const summaryId = `exception-${key}-summary`;
          const summary = exceptionSummary(repo);
          return (
            <div className={`exception ${isSelected ? "exception--selected" : ""}`} key={key}>
              <button
                type="button"
                id={`exception-${key}`}
                ref={(el) => {
                  headerRefs.current[index] = el;
                }}
                className="exception__header"
                aria-expanded={expanded}
                aria-controls={expanded ? bodyId : undefined}
                aria-describedby={summary ? summaryId : undefined}
                onClick={() => toggle(key)}
                onFocus={() => setSelectedKey(key)}
                onKeyDown={(e) => onHeaderKey(e, key)}
              >
                <Icon
                  name="disclosure"
                  size={9}
                  className={`exception__disclosure ${expanded ? "exception__disclosure--open" : ""}`}
                />
                <span className="exception__name">{repoName(repo)}</span>
                {/* Read as the button's description, so its name stays the repository name. */}
                <span className="exception__summary" id={summaryId} aria-hidden="true">
                  {summary}
                </span>
              </button>
              {expanded ? (
                <div className="exception__body" id={bodyId} role="group" aria-label={`Filters for ${name}`}>
                  <FilterRows
                    name={name}
                    filters={filtersOf(repo)}
                    inherited={orgResolved}
                    disabled={readOnly}
                    onChange={(patch) => saveRepo(repo, patch)}
                  />
                </div>
              ) : null}
            </div>
          );
        })}
        <div className="list-bar">
          <div className="add-remove">
            <button
              type="button"
              onClick={() => setAddOpen(true)}
              disabled={readOnly || candidates.length === 0}
              title="Add Exceptions"
              aria-label="Add Exceptions…"
            >
              <Icon name="plus" size={13} />
            </button>
            <button
              type="button"
              onClick={() => selected && removeException(selected)}
              disabled={readOnly || !selected}
              title="Remove Exception"
              aria-label="Remove Exception"
            >
              <Icon name="minus" size={13} />
            </button>
          </div>
        </div>
      </FormSection>

      {addOpen ? (
        <AddExceptionsSheet
          repos={candidates}
          labelOf={accountIds.size > 1 ? labelOf : undefined}
          onCancel={() => setAddOpen(false)}
          onAdd={addExceptions}
        />
      ) : null}
    </div>
  );
}
