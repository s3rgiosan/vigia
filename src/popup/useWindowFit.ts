import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { useEffect, useRef, type RefObject } from "react";
import { isPreview } from "../lib/preview";

/** Panel width and the height range the window follows the content within. */
const PANEL_WIDTH = 380;
const MIN_HEIGHT = 200;
const MAX_HEIGHT = 520;

/** What the panel currently shows, which decides the elements that are measured. */
export type FitLayout = "blank" | "empty" | "list";

function part(panel: HTMLElement, name: string): HTMLElement | null {
  return panel.querySelector<HTMLElement>(`[data-fit="${name}"]`);
}

/**
 * Content height of the panel. With a list, the window's current height with the list's visible
 * area swapped for its full content, so the header, banners and footer count as laid out.
 */
function contentHeight(panel: HTMLElement): number {
  const list = part(panel, "list");
  const inner = part(panel, "inner");
  if (list && inner) {
    const style = getComputedStyle(list);
    const padding = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
    const visible = list.clientHeight - padding;
    return window.innerHeight - visible + inner.offsetHeight;
  }
  const empty = part(panel, "empty");
  return (empty ? empty.offsetHeight : 0) + 64;
}

/**
 * Resizes the window to the panel's content, within the allowed range. The panel is measured when
 * the layout changes and whenever a `data-fit` element resizes.
 */
export function useWindowFit(panelRef: RefObject<HTMLElement | null>, layout: FitLayout): void {
  const lastHeight = useRef(0);

  useEffect(() => {
    const panel = panelRef.current;
    if (isPreview() || !panel) {
      return;
    }
    const fit = () => {
      const height = Math.ceil(Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, contentHeight(panel))));
      if (height !== lastHeight.current) {
        lastHeight.current = height;
        getCurrentWindow()
          .setSize(new LogicalSize(PANEL_WIDTH, height))
          .catch(console.error);
      }
    };
    fit();
    const observer = new ResizeObserver(fit);
    for (const element of panel.querySelectorAll("[data-fit]")) {
      observer.observe(element);
    }
    return () => observer.disconnect();
  }, [panelRef, layout]);
}
