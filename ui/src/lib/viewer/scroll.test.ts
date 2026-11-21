import { describe, expect, it } from 'vitest';

import { MAX_SPACER, moveOf, positionAt, scrollRange, scrollTopAt, Wheel } from './scroll';

const GIB = 1024 ** 3;

describe('scrollRange', () => {
  it('is a pixel per byte until the spacer is at its cap', () => {
    expect(scrollRange(5_000, 600)).toBe(5_000);
    expect(scrollRange(0, 600)).toBe(0);
    expect(scrollRange(-3, 600)).toBe(0);
    expect(scrollRange(100 * GIB, 600)).toBe(MAX_SPACER - 600);
    expect(scrollRange(100 * GIB, 600) + 600).toBe(MAX_SPACER);
  });
});

describe('positionAt and scrollTopAt', () => {
  const size = 5 * GIB;
  const range = scrollRange(size, 600);

  it('are exact at both ends', () => {
    expect(positionAt(0, size, range)).toBe(0);
    expect(positionAt(range, size, range)).toBe(size);
    expect(positionAt(range + 40, size, range)).toBe(size);
    expect(scrollTopAt(0, size, range)).toBe(0);
    expect(scrollTopAt(size, size, range)).toBe(range);
  });

  it('never decrease with the scroll top (and the other way round)', () => {
    let position = -1;
    for (let top = 0; top <= range; top += 7_919) {
      const next = positionAt(top, size, range);
      expect(next).toBeGreaterThanOrEqual(position);
      position = next;
    }

    let top = -1;
    for (let at = 0; at <= size; at += 12_345_678) {
      const next = scrollTopAt(at, size, range);
      expect(next).toBeGreaterThanOrEqual(top);
      top = next;
    }
  });

  it('go there and back to within the bytes of one pixel', () => {
    const perPixel = size / range;
    for (const at of [1, 123_456, GIB, 3 * GIB + 17, size - 1]) {
      const back = positionAt(scrollTopAt(at, size, range), size, range);
      expect(Math.abs(back - at)).toBeLessThanOrEqual(perPixel);
    }
  });

  it('a file that fits has a range of nothing, whose only position is the start', () => {
    expect(positionAt(30, 100, 0)).toBe(0);
    expect(scrollTopAt(50, 100, 0)).toBe(0);
    expect(positionAt(30, 0, 0)).toBe(0);
  });

  it('a small file maps one pixel to one byte', () => {
    expect(positionAt(1234, 5_000, 5_000)).toBe(1234);
    expect(scrollTopAt(1234, 5_000, 5_000)).toBe(1234);
  });
});

describe('moveOf', () => {
  const press = (key: string, init: Partial<KeyboardEvent> = {}) =>
    moveOf({ key, ctrlKey: false, altKey: false, metaKey: false, shiftKey: false, ...init });

  it('names the viewing keys', () => {
    expect(press('ArrowUp')).toBe('lineUp');
    expect(press('ArrowDown')).toBe('lineDown');
    expect(press('PageUp')).toBe('pageUp');
    expect(press('PageDown')).toBe('pageDown');
    expect(press(' ')).toBe('pageDown');
    expect(press(' ', { shiftKey: true })).toBe('pageUp');
    expect(press('Home')).toBe('home');
    expect(press('End')).toBe('end');
  });

  it('leaves other keys and modified ones alone', () => {
    expect(press('a')).toBeUndefined();
    expect(press('ArrowDown', { ctrlKey: true })).toBeUndefined();
    expect(press('Home', { altKey: true })).toBeUndefined();
    expect(press('End', { metaKey: true })).toBeUndefined();
  });
});

describe('Wheel', () => {
  it('turns pixels into rows and keeps what is less than a row', () => {
    const wheel = new Wheel();

    expect(wheel.rows({ deltaY: 54, deltaMode: 0 }, 18)).toBe(3);
    expect(wheel.rows({ deltaY: 10, deltaMode: 0 }, 18)).toBe(0);
    expect(wheel.rows({ deltaY: 10, deltaMode: 0 }, 18)).toBe(1);
    expect(new Wheel().rows({ deltaY: -36, deltaMode: 0 }, 18)).toBe(-2);
  });

  it('takes a wheel that counts in lines as it is', () => {
    expect(new Wheel().rows({ deltaY: 3, deltaMode: 1 }, 18)).toBe(3);
  });
});
