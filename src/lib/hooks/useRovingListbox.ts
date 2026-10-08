import { useCallback } from "react";
import { useLatest } from "./useLatest";

interface KeyLike {
  key: string;
  preventDefault: () => void;
}

export interface RovingListboxOptions {
  count: number;
  activeIndex: number;
  onMove: (index: number) => void;
  onActivate?: (index: number) => void;
  /** Rows skipped by PageUp and PageDown. */
  pageSize?: number;
}

/**
 * Returns a keydown handler for a listbox with one active option: arrows, Home, End, PageUp and
 * PageDown move it, Space and Enter activate it.
 */
export function useRovingListbox(options: RovingListboxOptions): (event: KeyLike) => void {
  const latest = useLatest(options);

  return useCallback(
    (event: KeyLike) => {
      const { count, activeIndex, onMove, onActivate, pageSize = 10 } = latest.current;
      if (count <= 0) {
        return;
      }
      const clamp = (index: number) => Math.min(count - 1, Math.max(0, index));

      switch (event.key) {
        case "ArrowDown":
          onMove(clamp(activeIndex + 1));
          break;
        case "ArrowUp":
          onMove(clamp(activeIndex - 1));
          break;
        case "Home":
          onMove(0);
          break;
        case "End":
          onMove(count - 1);
          break;
        case "PageDown":
          onMove(clamp(activeIndex + pageSize));
          break;
        case "PageUp":
          onMove(clamp(activeIndex - pageSize));
          break;
        case " ":
        case "Enter":
          if (!onActivate) {
            return;
          }
          onActivate(activeIndex);
          break;
        default:
          return;
      }
      event.preventDefault();
    },
    [latest],
  );
}
