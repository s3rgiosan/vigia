import { useEffect, type RefObject } from "react";
import { useLatest } from "../lib/hooks/useLatest";

export interface PopupShortcuts {
  filterRef: RefObject<HTMLInputElement | null>;
  setFilter: (value: string) => void;
  refresh: () => void;
  openSettings: () => void;
  hide: () => void;
}

/**
 * Window-level shortcuts: ⌘R refreshes, ⌘, opens Settings, ⌘F focuses the filter, Esc clears the
 * filter or closes the popup, and typing anywhere starts filtering, like type-to-select in Finder.
 */
export function usePopupShortcuts(options: PopupShortcuts): void {
  const latest = useLatest(options);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const { filterRef, setFilter, refresh, openSettings, hide } = latest.current;
      const input = filterRef.current;
      const typingInFilter = document.activeElement === input;
      const plainCharacter = e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey && e.key !== " ";
      if (e.metaKey && e.key === "r") {
        e.preventDefault();
        refresh();
      } else if (e.metaKey && e.key === ",") {
        e.preventDefault();
        openSettings();
      } else if (e.metaKey && e.key === "f") {
        e.preventDefault();
        input?.focus();
        input?.select();
      } else if (e.key === "Escape") {
        e.preventDefault();
        const filter = input?.value ?? "";
        if (filter !== "") {
          setFilter("");
          input?.focus();
        } else {
          hide();
        }
      } else if (!typingInFilter && plainCharacter) {
        input?.focus();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [latest]);
}
