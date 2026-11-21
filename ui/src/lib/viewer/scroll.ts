/**
 * Scaled scrolling. A browser cannot scroll a box that is billions of pixels high (the largest ones are about 33
 * million pixels, and fractions get lost long before), so the scroll bar of a big file runs over a spacer of at
 * most {@link MAX_SPACER} pixels and a pixel stands for a whole number of bytes. Everything here is arithmetic on
 * numbers: a position is a byte offset in `0..=extent`, a scroll top is pixels in `0..=range`, and the two maps are
 * monotonic and exact at both ends (the top is the start of the file, the bottom is `extent`).
 */

/** The tallest spacer the scroll bar runs over, in pixels. */
export const MAX_SPACER = 8_000_000;

/**
 * How far the scroll bar can scroll (spacer height minus the viewport's): one pixel per byte of `extent`, up to the
 * spacer's cap. A file that fits the viewport has no range.
 */
export function scrollRange(extent: number, viewport: number): number {
  const room = Math.max(0, MAX_SPACER - Math.max(0, viewport));

  return Math.min(Math.max(0, Math.floor(extent)), room);
}

/** The position (a byte offset) a scroll top stands for. The bottom is `extent` itself; nothing goes past it. */
export function positionAt(scrollTop: number, extent: number, range: number): number {
  if (range <= 0 || extent <= 0 || scrollTop <= 0) {
    return 0;
  }

  if (scrollTop >= range) {
    return extent;
  }

  return Math.min(extent, Math.floor((scrollTop / range) * extent));
}

/** The scroll top a position stands for. The inverse of {@link positionAt}, up to the bytes one pixel holds. */
export function scrollTopAt(position: number, extent: number, range: number): number {
  if (range <= 0 || extent <= 0 || position <= 0) {
    return 0;
  }

  if (position >= extent) {
    return range;
  }

  return Math.round((position / extent) * range);
}

/** What a viewing key asks for. */
export type Move = 'lineUp' | 'lineDown' | 'pageUp' | 'pageDown' | 'home' | 'end';

/** The move a key stands for in the text and hex views (arrows, pages, Space and Shift+Space, Home, End). */
export function moveOf(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'altKey' | 'metaKey' | 'shiftKey'>): Move | undefined {
  if (event.ctrlKey || event.altKey || event.metaKey) {
    return undefined;
  }

  switch (event.key) {
    case 'ArrowUp':
      return 'lineUp';
    case 'ArrowDown':
      return 'lineDown';
    case 'PageUp':
      return 'pageUp';
    case 'PageDown':
      return 'pageDown';
    case ' ':
      return event.shiftKey ? 'pageUp' : 'pageDown';
    case 'Home':
      return 'home';
    case 'End':
      return 'end';
    default:
      return undefined;
  }
}

/** Turns wheel events into whole rows, keeping what is less than a row for the next event (a trackpad sends many). */
export class Wheel {
  #rest = 0;

  rows(event: Pick<WheelEvent, 'deltaY' | 'deltaMode'>, rowHeight: number): number {
    this.#rest += event.deltaMode === 1 ? event.deltaY : event.deltaY / rowHeight;
    const rows = Math.trunc(this.#rest);
    this.#rest -= rows;

    return rows;
  }
}
