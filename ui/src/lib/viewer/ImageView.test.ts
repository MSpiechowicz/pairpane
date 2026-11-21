import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ViewKind } from '$lib/ipc/bindings/ViewKind';

import { FakeFile, openedSession } from '../../../tests/view';
import ImageView from './ImageView.svelte';

const MIB = 1024 * 1024;

const create = vi.fn<(blob: Blob) => string>();
const revoke = vi.fn<(url: string) => void>();
const original = { create: URL.createObjectURL, revoke: URL.revokeObjectURL };

beforeEach(() => {
  create.mockReset().mockReturnValue('blob:image-1');
  revoke.mockReset();
  URL.createObjectURL = create as typeof URL.createObjectURL;
  URL.revokeObjectURL = revoke;
});

afterEach(() => {
  URL.createObjectURL = original.create;
  URL.revokeObjectURL = original.revoke;
});

/** A file of `size` bytes (each one `index % 251`) that the backend says is a PNG. */
async function open(size: number) {
  const bytes = Uint8Array.from({ length: size }, (_, index) => index % 251);
  const world = await openedSession(new FakeFile(bytes), () => ({ kind: 'image', mime: 'image/png' }));
  const view = render(ImageView, { session: world.session });

  return { ...world, view, bytes };
}

describe('ImageView', () => {
  it('reads the file in pieces of at most 4 MiB and shows it, typed as the backend says', async () => {
    const { sent, bytes } = await open(9 * MIB);

    const image = await screen.findByRole('img', { name: 'file.txt' });

    expect(sent('view_bytes')).toEqual([
      { session: 1, offset: 0, length: 4 * MIB },
      { session: 1, offset: 4 * MIB, length: 4 * MIB },
      { session: 1, offset: 8 * MIB, length: 1 * MIB },
    ]);
    expect(create).toHaveBeenCalledTimes(1);
    const blob = create.mock.calls[0][0];
    expect(blob.type).toBe('image/png');
    expect(blob.size).toBe(bytes.length);
    expect(new Uint8Array(await blob.arrayBuffer()).subarray(4 * MIB - 2, 4 * MIB + 2)).toEqual(
      bytes.subarray(4 * MIB - 2, 4 * MIB + 2),
    );
    expect(image).toHaveAttribute('src', 'blob:image-1');
  });

  it('revokes the object URL when the view goes', async () => {
    const { view } = await open(1000);
    await screen.findByRole('img');
    expect(revoke).not.toHaveBeenCalled();

    view.unmount();

    expect(revoke).toHaveBeenCalledWith('blob:image-1');
  });

  it('makes no object URL when the view goes after the last piece was read, so there is none to leak', async () => {
    const { view } = await open(0); // nothing to read: the file is "whole" as soon as the read starts

    view.unmount();
    await new Promise((resolve) => setTimeout(resolve, 10));

    expect(create).not.toHaveBeenCalled();
  });

  it('says so when the webview cannot decode the image', async () => {
    await open(1000);

    await fireEvent.error(await screen.findByRole('img'));

    expect(screen.getByRole('alert')).toHaveTextContent('This image cannot be shown here');
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
  });

  it('does not read an image over 64 MiB', async () => {
    const { sent } = await open(64 * MIB + 1);

    expect(await screen.findByRole('alert')).toHaveTextContent('This image is too big to show here');
    expect(sent('view_bytes')).toHaveLength(0);
    expect(create).not.toHaveBeenCalled();
  });

  it('shows the backend`s words when a read fails', async () => {
    const world = await openedSession(new FakeFile('x'), () => ({ kind: 'image', mime: 'image/png' }), {
      view_bytes: () => {
        throw { code: 'io', message: 'The file changed' };
      },
    });
    render(ImageView, { session: world.session });

    expect(await screen.findByRole('alert')).toHaveTextContent('The file changed');
  });

  describe('a HEIC image the webview cannot show', () => {
    const createImageData = vi.fn<(width: number, height: number) => { data: Uint8ClampedArray }>();
    const putImageData = vi.fn<(image: { data: Uint8ClampedArray }, x: number, y: number) => void>();
    const getContext = HTMLCanvasElement.prototype.getContext;

    beforeEach(() => {
      createImageData.mockReset().mockImplementation((width, height) => ({ data: new Uint8ClampedArray(width * height * 4) }));
      putImageData.mockReset();
      HTMLCanvasElement.prototype.getContext = (() => ({ createImageData, putImageData })) as never;
    });

    afterEach(() => {
      HTMLCanvasElement.prototype.getContext = getContext;
    });

    /** A `view_image_decode` answer: the size, then `width * height` pixels of `fill`. */
    function decodedAnswer(width: number, height: number, fill = 7): ArrayBuffer {
      const bytes = new Uint8Array(8 + width * height * 4).fill(fill, 8);
      const header = new DataView(bytes.buffer);
      header.setUint32(0, width, true);
      header.setUint32(4, height, true);

      return bytes.buffer;
    }

    async function openHeic(decode: () => unknown, mime = 'image/heic') {
      const kind = (): ViewKind => ({ kind: 'image', mime });
      const world = await openedSession(new FakeFile(new Uint8Array(100)), kind, { view_image_decode: decode });
      const view = render(ImageView, { session: world.session });

      return { ...world, view };
    }

    it('is decoded by the backend and drawn on a canvas', async () => {
      const { sent } = await openHeic(() => decodedAnswer(3, 2));

      await fireEvent.error(await screen.findByRole('img'));
      const canvas = await screen.findByRole('img', { name: 'file.txt' });

      expect(canvas.tagName).toBe('CANVAS');
      expect(canvas).toHaveAttribute('width', '3');
      expect(canvas).toHaveAttribute('height', '2');
      await vi.waitFor(() => expect(putImageData).toHaveBeenCalledTimes(1));
      expect(createImageData).toHaveBeenCalledWith(3, 2);
      const [image, x, y] = putImageData.mock.calls[0];
      expect(Array.from(image.data)).toEqual(new Array<number>(3 * 2 * 4).fill(7));
      expect([x, y]).toEqual([0, 0]);
      expect(sent('view_image_decode')).toEqual([{ session: 1 }]);
      expect(revoke).toHaveBeenCalledWith('blob:image-1');
      expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    });

    it('is not decoded by the backend when the webview shows it', async () => {
      const { sent } = await openHeic(() => decodedAnswer(1, 1));

      expect(await screen.findByRole('img')).toHaveAttribute('src', 'blob:image-1');

      expect(sent('view_image_decode')).toHaveLength(0);
    });

    it('is not asked of the backend for another image type', async () => {
      const { sent } = await openHeic(() => decodedAnswer(1, 1), 'image/png');

      await fireEvent.error(await screen.findByRole('img'));

      expect(screen.getByRole('alert')).toHaveTextContent('This image cannot be shown here');
      expect(sent('view_image_decode')).toHaveLength(0);
    });

    it('says there is no decoder, with the backend`s hint', async () => {
      await openHeic(() => {
        throw { code: 'unsupported', message: 'no heic decoder on this system: install libheif with an hevc plugin' };
      });

      await fireEvent.error(await screen.findByRole('img'));

      const alert = await screen.findByRole('alert');
      expect(alert).toHaveTextContent('No HEIC decoder on this system: install libheif with an hevc plugin');
      expect(screen.queryByRole('img')).not.toBeInTheDocument();
    });

    it('says only that it cannot be shown when there is no decoder and no hint', async () => {
      await openHeic(() => {
        throw { code: 'unsupported', message: 'no heic decoder on this system' };
      });

      await fireEvent.error(await screen.findByRole('img'));

      expect(await screen.findByRole('alert')).toHaveTextContent('This image cannot be shown here');
    });

    it('says the picture is too big when the webview refuses to draw it', async () => {
      createImageData.mockImplementation(() => {
        throw new RangeError('out of memory');
      });
      await openHeic(() => decodedAnswer(3, 2));

      await fireEvent.error(await screen.findByRole('img'));

      expect(await screen.findByRole('alert')).toHaveTextContent('This image is too big to show here');
      expect(screen.queryByRole('img')).not.toBeInTheDocument();
    });

    it('shows the backend`s words when it cannot decode the image', async () => {
      await openHeic(() => {
        throw { code: 'io', message: 'the image cannot be decoded' };
      });

      await fireEvent.error(await screen.findByRole('img'));

      expect(await screen.findByRole('alert')).toHaveTextContent('the image cannot be decoded');
    });

    it('says so when the decoded pixels do not fit their size', async () => {
      await openHeic(() => decodedAnswer(3, 2).slice(0, 20));

      await fireEvent.error(await screen.findByRole('img'));

      expect(await screen.findByRole('alert')).toHaveTextContent('This image cannot be shown here');
      expect(putImageData).not.toHaveBeenCalled();
    });

    it('is not drawn when the view goes while the backend decodes', async () => {
      let answer: (buffer: ArrayBuffer) => void = () => undefined;
      const pending = new Promise<ArrayBuffer>((resolve) => (answer = resolve));
      const { view, sent } = await openHeic(() => pending);

      await fireEvent.error(await screen.findByRole('img'));
      await vi.waitFor(() => expect(sent('view_image_decode')).toHaveLength(1));
      view.unmount();
      answer(decodedAnswer(3, 2));
      await pending;
      await new Promise((resolve) => setTimeout(resolve, 0)); // let the decode's continuation run

      expect(putImageData).not.toHaveBeenCalled();
    });
  });
});
