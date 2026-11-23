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
