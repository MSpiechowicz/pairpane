/**
 * Where a list scrolled to `offset`, showing `size` along its axis, has to scroll so that the span from `start` to
 * `end` is in view: the same offset when it already is, the least move otherwise.
 */
export function revealed(offset: number, size: number, start: number, end: number): number {
  if (start < offset) {
    return start;
  }

  if (end > offset + size) {
    return end - size;
  }

  return offset;
}

/** The numbers from `first` up to, not including, `last` (none when `last` is not above `first`). */
export function span(first: number, last: number): number[] {
  return Array.from({ length: Math.max(0, last - first) }, (_, offset) => first + offset);
}

/** A run of items: from `first` up to, not including, `last`. */
export interface Range {
  first: number;
  last: number;
}

/** How many items of at least `min` fit along `size`: always one at least. */
export function fits(size: number, min: number): number {
  return Math.max(1, Math.floor(size / min));
}

/**
 * The items of a list of `total` that show when it is scrolled to `offset` with `size` in view, each `item` long, and
 * `overscan` more on each side.
 */
export function windowOf(offset: number, size: number, item: number, overscan: number, total: number): Range {
  return {
    first: Math.max(0, Math.floor(offset / item) - overscan),
    last: Math.min(total, Math.ceil((offset + size) / item) + overscan),
  };
}

/**
 * Scrolls `view` along its vertical (`'y'`) or horizontal (`'x'`) axis by the least that brings the span from `start`
 * to `end` into view. Returns where it is scrolled to, or `undefined` while it has no size (it is not laid out yet).
 */
export function reveal(view: HTMLElement | undefined, axis: 'x' | 'y', start: number, end: number): number | undefined {
  if (view === undefined) {
    return undefined;
  }

  if (axis === 'y') {
    if (view.clientHeight === 0) {
      return undefined;
    }

    view.scrollTop = revealed(view.scrollTop, view.clientHeight, start, end);
    return view.scrollTop;
  }

  if (view.clientWidth === 0) {
    return undefined;
  }

  view.scrollLeft = revealed(view.scrollLeft, view.clientWidth, start, end);
  return view.scrollLeft;
}
