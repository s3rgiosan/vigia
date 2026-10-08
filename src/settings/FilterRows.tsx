import { FormRow } from "../components/Form";
import { PopUpButton } from "../components/PopUpButton";
import type { OrgFilters } from "../lib/tauri";
import {
  MODE_OPTIONS,
  modeOf,
  patternPlaceholder,
  patternsOfMode,
  tagChoiceOf,
  tagOptions,
  tagOverrideOf,
  type InheritedFilters,
} from "./filters";
import { PatternField } from "./PatternField";

/**
 * The Branches, Ignored workflows and Tag runs rows of an organization or repository. Default
 * disables the field and shows the inherited value as its placeholder; Custom enables it, and an
 * empty Custom field means "default branch only" or "ignore nothing". `name` names every control.
 */
export function FilterRows({
  name,
  filters,
  inherited,
  disabled,
  onChange,
}: {
  name: string;
  filters: OrgFilters;
  inherited: InheritedFilters;
  disabled: boolean;
  onChange: (patch: Partial<OrgFilters>) => void;
}) {
  const branchMode = modeOf(filters.branch_patterns);
  const ignoredMode = modeOf(filters.ignored_workflows);

  return (
    <>
      <FormRow label="Branches">
        <PopUpButton
          label={`Branches filter mode for ${name}`}
          value={branchMode}
          options={MODE_OPTIONS}
          disabled={disabled}
          onChange={(mode) => onChange({ branch_patterns: patternsOfMode(mode) })}
        />
        <PatternField
          label={`Branches for ${name}`}
          value={filters.branch_patterns ?? []}
          placeholder={patternPlaceholder(branchMode, inherited.branch_patterns, "Default branch")}
          disabled={disabled || branchMode === "default"}
          onSave={(patterns) => onChange({ branch_patterns: patterns })}
        />
      </FormRow>
      <FormRow label="Ignored workflows">
        <PopUpButton
          label={`Ignored workflows filter mode for ${name}`}
          value={ignoredMode}
          options={MODE_OPTIONS}
          disabled={disabled}
          onChange={(mode) => onChange({ ignored_workflows: patternsOfMode(mode) })}
        />
        <PatternField
          label={`Ignored workflows for ${name}`}
          value={filters.ignored_workflows ?? []}
          placeholder={patternPlaceholder(ignoredMode, inherited.ignored_workflows, "None")}
          disabled={disabled || ignoredMode === "default"}
          onSave={(patterns) => onChange({ ignored_workflows: patterns })}
        />
      </FormRow>
      <FormRow label="Tag runs">
        <PopUpButton
          label={`Tag runs for ${name}`}
          value={tagChoiceOf(filters.include_tags)}
          options={tagOptions(inherited.include_tags)}
          disabled={disabled}
          onChange={(choice) => onChange({ include_tags: tagOverrideOf(choice) })}
        />
      </FormRow>
    </>
  );
}
