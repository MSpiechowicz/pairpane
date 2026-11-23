import { describe, expect, it } from 'vitest';

import { fits, revealed, span, windowOf } from './scroll';

describe('fits', () => {
  it('counts the whole items that fit, and at least one', () => {
    expect(fits(400, 136)).toBe(2);
    expect(fits(408, 136)).toBe(3);
    expect(fits(100, 136)).toBe(1);
    expect(fits(0, 136)).toBe(1);
  });
});

describe('windowOf', () => {
  it('has the items in view and the overscan on each side, within the list', () => {
    expect(windowOf(0, 100, 10, 2, 50)).toEqual({ first: 0, last: 12 });
    expect(windowOf(200, 100, 10, 2, 50)).toEqual({ first: 18, last: 32 });
    expect(windowOf(450, 100, 10, 2, 50)).toEqual({ first: 43, last: 50 });
  });

  it('counts an item that is only partly in view', () => {
    expect(windowOf(5, 100, 10, 0, 50)).toEqual({ first: 0, last: 11 });
  });

  it('has no items in an empty list', () => {
    expect(windowOf(0, 100, 10, 2, 0)).toEqual({ first: 0, last: 0 });
  });
});

describe('revealed and span', () => {
  it('scrolls by the least that brings a span into view', () => {
    expect(revealed(100, 50, 110, 130)).toBe(100);
    expect(revealed(100, 50, 80, 90)).toBe(80);
    expect(revealed(100, 50, 160, 180)).toBe(130);
  });

  it('lists the numbers from first up to last', () => {
    expect(span(3, 6)).toEqual([3, 4, 5]);
    expect(span(6, 3)).toEqual([]);
  });
});
