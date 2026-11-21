/** An image the backend decoded: `pixels` are RGBA, four bytes per pixel, rows one after another. */
export interface Decoded {
  width: number;
  height: number;
  pixels: Uint8ClampedArray;
}

const HEADER = 8;

/** Reads a `view_image_decode` answer: width and height (u32, little-endian), then exactly width * height RGBA pixels. */
export function parseDecoded(buffer: ArrayBuffer): Decoded {
  if (buffer.byteLength < HEADER) {
    throw new Error('The decoded image is cut short');
  }

  const header = new DataView(buffer);
  const width = header.getUint32(0, true);
  const height = header.getUint32(4, true);
  if (width === 0 || height === 0) {
    throw new Error('The decoded image has no size');
  }

  if (buffer.byteLength !== HEADER + width * height * 4) {
    throw new Error('The decoded image does not fit its size');
  }

  return { width, height, pixels: new Uint8ClampedArray(buffer, HEADER) };
}
