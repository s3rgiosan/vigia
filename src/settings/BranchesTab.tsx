import { useRef, useState } from "react";
import { FormRow, FormSection, Toggle } from "../components/Form";
import { Icon } from "../components/Icon";
import { useRovingListbox } from "../lib/hooks/useRovingListbox";
import { useSafeStorage } from "../lib/hooks/useSafeStorage";
import type { Organization } from "../lib/tauri";
import { hasOverrides, orgKey, type InheritedFilters } from "./filters";
import { OrgDetail } from "./OrgDetail";
import { PatternField } from "./PatternField";
import { useSettings } from "./SettingsContext";

/** The remembered sidebar selection: ALL_KEY or an organization's key. */
const SCOPE_KEY = "vigia.settings.filters.scope";
const ALL_KEY = "all";

const collator = new Intl.Collator(undefined, { numeric: true });

/** Organizations sorted by host, then owner, with numbers in names compared by value. */
function sortOrganizations(orgs: Organization[]): Organization[] {
  return [...orgs].sort((a, b) => collator.compare(a.host, b.host) || collator.compare(a.owner, b.owner));
}

/** Sorted organizations grouped under their hosts. */
function groupByHost(orgs: Organization[]): { host: string; orgs: Organization[] }[] {
  const groups: { host: string; orgs: Organization[] }[] = [];
  for (const org of orgs) {
    const last = groups[groups.length - 1];
    if (last?.host === org.host) {
      last.orgs.push(org);
    } else {
      groups.push({ host: org.host, orgs: [org] });
    }
  }
  return groups;
}

export function BranchesTab() {
  const { view, readOnly, requestAdd } = useSettings();
  const [storedKey, storeKey] = useSafeStorage(SCOPE_KEY, ALL_KEY);
  const [sessionAdded, setSessionAdded] = useState<string[]>([]);
  const optionRefs = useRef<(HTMLLIElement | null)[]>([]);

  const orgs = sortOrganizations(view.organizations);
  const keys = [ALL_KEY, ...orgs.map(orgKey)];
  // A remembered organization that no longer has watched repositories falls back to All Repositories.
  const selectedKey = keys.includes(storedKey) ? storedKey : ALL_KEY;
  const selectedOrg = orgs.find((org) => orgKey(org) === selectedKey) ?? null;
  const global: InheritedFilters = {
    branch_patterns: view.settings.branch_patterns,
    ignored_workflows: view.settings.ignored_workflows ?? [],
    include_tags: view.settings.include_tags ?? false,
  };

  const onListKey = useRovingListbox({
    count: keys.length,
    activeIndex: keys.indexOf(selectedKey),
    onMove: (index) => {
      storeKey(keys[index]);
      optionRefs.current[index]?.focus();
    },
  });

  function option(key: string, index: number, label: string, custom: boolean) {
    const isSelected = key === selectedKey;
    return (
      <li
        key={key}
        ref={(el) => {
          optionRefs.current[index] = el;
        }}
        role="option"
        aria-selected={isSelected}
        tabIndex={isSelected ? 0 : -1}
        className={`source-list__row ${isSelected ? "source-list__row--selected" : ""}`}
        onClick={() => storeKey(key)}
      >
        <span className="source-list__text">
          <span>{label}</span>
        </span>
        {custom ? <span className="source-list__badge">Custom</span> : null}
      </li>
    );
  }

  if (view.accounts.length === 0) {
    return (
      <div className="empty-pane">
        <Icon name="branch" size={28} />
        <h2>No accounts</h2>
        <p>Add an account and choose repositories before setting which branches to watch.</p>
        <button type="button" className="default" onClick={requestAdd} disabled={readOnly || view.secrets_blocked}>
          Add Account…
        </button>
      </div>
    );
  }

  return (
    <div className="accounts filters">
      <div className="accounts__sidebar">
        <ul className="source-list" role="listbox" aria-label="Filters" onKeyDown={onListKey}>
          {option(ALL_KEY, 0, "All Repositories", false)}
          {groupByHost(orgs).map((group) => {
            const headerId = `filters-host-${group.host}`;
            return (
              <li key={group.host} role="presentation">
                <div className="source-list__header" id={headerId}>
                  {group.host}
                </div>
                <ul className="source-list__group" role="group" aria-labelledby={headerId}>
                  {group.orgs.map((org) => {
                    const key = orgKey(org);
                    return option(key, keys.indexOf(key), org.owner, hasOverrides(org.filters));
                  })}
                </ul>
              </li>
            );
          })}
        </ul>
      </div>

      {selectedOrg ? (
        <OrgDetail
          key={selectedKey}
          org={selectedOrg}
          global={global}
          sessionAdded={sessionAdded}
          onSessionAdd={(added) => setSessionAdded((current) => [...current, ...added])}
          onSessionRemove={(key) => setSessionAdded((current) => current.filter((k) => k !== key))}
        />
      ) : (
        <GlobalDetail global={global} />
      )}
    </div>
  );
}

/** The filters every repository uses unless its organization or the repository overrides them. */
function GlobalDetail({ global }: { global: InheritedFilters }) {
  const { saveSettings, readOnly } = useSettings();

  return (
    <div className="accounts__detail">
      <FormSection
        title="All repositories"
        footer="Separate patterns with commas, such as main, release/*. Matching ignores case."
      >
        <FormRow label="Branches" htmlFor="global-branches" description="Empty watches each repository’s default branch.">
          <PatternField
            id="global-branches"
            wide
            value={global.branch_patterns}
            placeholder="Default branch"
            disabled={readOnly}
            onSave={(patterns) => saveSettings({ branch_patterns: patterns })}
          />
        </FormRow>
        <FormRow
          label="Ignored workflows"
          htmlFor="global-ignored-workflows"
          description="Hides matching workflows and pipelines, such as Dependabot*."
        >
          <PatternField
            id="global-ignored-workflows"
            wide
            value={global.ignored_workflows}
            placeholder="None"
            disabled={readOnly}
            onSave={(patterns) => saveSettings({ ignored_workflows: patterns })}
          />
        </FormRow>
        <FormRow label="Tag runs" htmlFor="global-include-tags" description="Counts the latest run started by a tag.">
          <Toggle
            id="global-include-tags"
            label="Include tag runs"
            checked={global.include_tags}
            disabled={readOnly}
            onChange={(include) => saveSettings({ include_tags: include })}
          />
        </FormRow>
      </FormSection>

      <FormSection title="Known limits">
        <ul className="limits">
          <li>On a busy repository, a branch or workflow that runs rarely can drop out of the checked runs.</li>
          <li>GitLab child pipelines aren’t checked, so a parent pipeline can pass while a child fails.</li>
          <li>An archived repository keeps its last state until you stop watching it.</li>
        </ul>
      </FormSection>
    </div>
  );
}
