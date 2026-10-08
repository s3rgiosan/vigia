// Renders a Settings pane inside a SettingsContext built from fabricated data.

import { render } from "@testing-library/react";
import type { ReactElement } from "react";
import { vi } from "vitest";
import { SettingsContext, type SettingsContextValue } from "../settings/SettingsContext";
import { makeView } from "./fixtures";

/** A context value with mock functions; `readOnly` follows the view unless given. */
export function makeSettingsContext(overrides: Partial<SettingsContextValue> = {}): SettingsContextValue {
  const view = overrides.view ?? makeView();
  return {
    view,
    snapshot: null,
    reload: vi.fn(async () => undefined),
    setError: vi.fn(),
    saveSettings: vi.fn(async () => undefined),
    readOnly: view.read_only,
    requestAdd: vi.fn(),
    ...overrides,
  };
}

/**
 * Renders `ui` with a settings context. `update` re-renders with a new view or snapshot and keeps
 * the same mock functions.
 */
export function renderWithSettings(ui: ReactElement, overrides: Partial<SettingsContextValue> = {}) {
  let value = makeSettingsContext(overrides);
  const result = render(<SettingsContext.Provider value={value}>{ui}</SettingsContext.Provider>);
  return {
    ...result,
    context: value,
    update(next: Partial<SettingsContextValue>, nextUi: ReactElement = ui) {
      const view = next.view ?? value.view;
      value = { ...value, readOnly: view.read_only, ...next, view };
      result.rerender(<SettingsContext.Provider value={value}>{nextUi}</SettingsContext.Provider>);
    },
  };
}
