// Fixed-height windowing for thread lists. Rows have one height per density (packages/tokens layout
// row heights), so the window is plain arithmetic: no measuring, no layout shift, 10,000 rows cost the same
// as 50.

export interface Window {
  start: number;
  /** Exclusive. */
  end: number;
  /** Total scroll height. */
  height: number;
}

export function windowFor(
  count: number,
  rowHeight: number,
  scrollTop: number,
  viewport: number,
  overscan = 8,
): Window {
  const height = count * rowHeight;
  if (count === 0 || rowHeight <= 0) return { start: 0, end: 0, height };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const visible = Math.ceil(Math.max(viewport, rowHeight) / rowHeight) + 1;
  return {
    start: Math.max(0, first - overscan),
    end: Math.min(count, first + visible + overscan),
    height,
  };
}

/** The scrollTop that brings row `index` into view with `margin` rows of context, or `null` if it already is. */
export function scrollToReveal(
  index: number,
  rowHeight: number,
  scrollTop: number,
  viewport: number,
  margin = 2,
): number | null {
  const top = index * rowHeight;
  const bottom = top + rowHeight;
  const pad = margin * rowHeight;
  if (top - pad < scrollTop) return Math.max(0, top - pad);
  if (bottom + pad > scrollTop + viewport) return bottom + pad - viewport;
  return null;
}

/** Whether to fetch the next page: the window reaches within `threshold` rows of the end. */
export function nearEnd(w: Window, count: number, threshold = 15): boolean {
  return count > 0 && w.end >= count - threshold;
}
