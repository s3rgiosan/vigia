/** Height in pixels of every row in the repository picker. */
export const ROW_HEIGHT = 26;

export interface RowWindow {
  /** Index of the first row to render. */
  start: number;
  /** Index after the last row to render. */
  end: number;
}

/** Rows to render for a scroll position, with a few extra rows on each side. */
export function visibleWindow(
  total: number,
  rowHeight: number,
  scrollTop: number,
  viewportHeight: number,
  overscan: number,
): RowWindow {
  if (total <= 0 || rowHeight <= 0) {
    return { start: 0, end: 0 };
  }
  const firstVisible = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const lastVisible = Math.ceil((Math.max(0, scrollTop) + Math.max(0, viewportHeight)) / rowHeight);
  const start = Math.min(Math.max(0, firstVisible - overscan), total);
  const end = Math.min(total, Math.max(start, lastVisible + overscan));
  return { start, end };
}
