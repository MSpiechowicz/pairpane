import { describe, expect, it } from 'vitest';

import { parseDecoded } from './decoded';

/** A `view_image_decode` answer: the header, then `pixels` bytes. */
function answer(width: number, height: number, pixels: number[]): ArrayBuffer {
  const bytes = new Uint8Array(8 + pixels.length);
  const header = new DataView(bytes.buffer);
  header.setUint32(0, width, true);
  header.setUint32(4, height, true);
  bytes.set(pixels, 8);

  return bytes.buffer;
}

describe('parseDecoded', () => {
  it('reads the size and the pixels', () => {
    const pixels = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

    const decoded = parseDecoded(answer(3, 1, pixels));

    expect(decoded.width).toBe(3);
    expect(decoded.height).toBe(1);
    expect(Array.from(decoded.pixels)).toEqual(pixels);
  });

  it('reads the size as little-endian', () => {
    const decoded = parseDecoded(answer(256, 1, new Array<number>(256 * 4).fill(0)));

    expect(decoded.width).toBe(256);
    expect(decoded.height).toBe(1);
  });

  it('refuses pixels that do not fill the size', () => {
    expect(() => parseDecoded(answer(2, 2, new Array<number>(15).fill(0)))).toThrow();
    expect(() => parseDecoded(answer(2, 2, new Array<number>(17).fill(0)))).toThrow();
  });

  it('refuses a size of zero', () => {
    expect(() => parseDecoded(answer(0, 2, []))).toThrow();
    expect(() => parseDecoded(answer(2, 0, []))).toThrow();
  });

  it('refuses an answer shorter than its header', () => {
    expect(() => parseDecoded(new ArrayBuffer(7))).toThrow();
  });
});
