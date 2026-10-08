import { createContext, useContext } from "react";
import type { Snapshot } from "../lib/snapshot";
import type { Settings, SettingsSheet, SettingsView } from "../lib/tauri";

/** What every Settings pane reads and changes. */
export interface SettingsContextValue {
  view: SettingsView;
  snapshot: Snapshot | null;
  /** Fetches the settings again after a change. */
  reload: () => Promise<void>;
  /** Shows a message in the window's error banner, or clears it with null. */
  setError: (message: string | null) => void;
  /** Saves a patch of the global settings, one after another. */
  saveSettings: (patch: Partial<Settings>) => Promise<void>;
  /** True when the config was written by a newer version and every control is disabled. */
  readOnly: boolean;
  /** Switches to the Accounts pane and opens the Add Account sheet. */
  requestAdd: () => void;
}

export const SettingsContext = createContext<SettingsContextValue | null>(null);

export function useSettings(): SettingsContextValue {
  const value = useContext(SettingsContext);
  if (!value) {
    throw new Error("useSettings needs a SettingsContext provider");
  }
  return value;
}

/** A sheet that a deep link, a `settings-open` event or another pane asks the Accounts pane to show. */
export interface SheetRequest {
  sheet: SettingsSheet;
  /** The account a Replace Token request applies to. */
  accountId: string | null;
}
