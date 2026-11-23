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

/** How a drawing went: the picture is on the canvas, or there is no 2D context, or the webview refused a picture this big. */
export type Drawn = 'drawn' | 'no-context' | 'too-big';

/** Draws `decoded` on `canvas`, which is as large as the picture. */
export function drawDecoded(canvas: HTMLCanvasElement, decoded: Decoded): Drawn {
  const context = canvas.getContext('2d');
  if (context === null) {
    return 'no-context';
  }

  try {
    const image = context.createImageData(decoded.width, decoded.height);
    image.data.set(decoded.pixels);
    context.putImageData(image, 0, 0);
  } catch {
    return 'too-big';
  }

  return 'drawn';
}
