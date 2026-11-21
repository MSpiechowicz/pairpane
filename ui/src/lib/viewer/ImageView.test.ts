import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

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
});
