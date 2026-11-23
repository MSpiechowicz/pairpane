import { describe, expect, it, vi } from 'vitest';

import { drawDecoded, parseDecoded } from './decoded';

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

describe('drawDecoded', () => {
  /** A canvas whose 2D context records what is drawn; `context` is what it gives. */
  function canvasWith(context: unknown) {
    const canvas = document.createElement('canvas');
    canvas.getContext = (() => context) as never;

    return canvas;
  }

  it('puts the pixels of a crafted answer on the canvas, as one image of the picture`s size', () => {
    const pixels = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0];
    const decoded = parseDecoded(answer(3, 1, pixels));
    const put = vi.fn<(image: { data: Uint8ClampedArray }, x: number, y: number) => void>();
    const context = { createImageData: (w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4) }), putImageData: put };

    expect(drawDecoded(canvasWith(context), decoded)).toBe('drawn');

    expect(put).toHaveBeenCalledTimes(1);
    expect(Array.from(put.mock.calls[0][0].data)).toEqual(pixels);
    expect(put.mock.calls[0].slice(1)).toEqual([0, 0]);
  });

  it('says so when there is no 2D context', () => {
    expect(drawDecoded(canvasWith(null), parseDecoded(answer(1, 1, [1, 2, 3, 4])))).toBe('no-context');
  });

  it('says so when the webview refuses a picture this big', () => {
    const context = {
      createImageData: () => {
        throw new Error('out of memory');
      },
    };

    expect(drawDecoded(canvasWith(context), parseDecoded(answer(1, 1, [1, 2, 3, 4])))).toBe('too-big');
  });
});
