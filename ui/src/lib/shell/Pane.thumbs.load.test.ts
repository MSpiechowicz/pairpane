import { fireEvent, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { PaneTabs } from '$lib/panes/tabs.svelte';

import { entry, fakeBackend } from '../../../tests/panes';
import {
  asked,
  held,
  mountThumbnails,
  picture,
  refusal,
  pending,
  sized,
  settle,
  thumbnails,
  tileAt,
  TILE_HEIGHT,
  viewport,
} from '../../../tests/thumbs';

afterEach(() => vi.restoreAllMocks());

describe('Pane thumbnails: what is asked for', () => {
  it('asks for the pictures only, with the folder, the name and the size 128', async () => {
    sized();
    const { backend } = await thumbnails({ thumb_get: () => Promise.reject(refusal('unsupported')) }, [
      entry('a.jpg'),
      entry('b.txt'),
      entry('c.PNG'),
      entry('d.pdf'),
      entry('album.jpg', 'dir'),
    ]);
    await settle();

    expect(asked(backend)).toEqual(['a.jpg', 'c.PNG']);
    expect(backend.invoke).toHaveBeenCalledWith('thumb_get', { path: '/big', name: 'a.jpg', size: 128 });
  });

  it('has three requests out at the most, the pictures in view first, then the ones near it', async () => {
    sized();
    const { answers, thumb_get } = held();
    const { backend } = await thumbnails({ thumb_get });
    await settle();

    expect(asked(backend)).toEqual(['p0.jpg', 'p1.jpg', 'p2.jpg']);

    for (let round = 0; round < 6; round += 1) {
      for (const [name, answer] of answers) {
        answer.resolve(picture(1, 1));
        answers.delete(name);
      }
      await settle();
      expect(answers.size).toBeLessThanOrEqual(3);
    }

    // Rows 0 to 2 are in view (items 2 to 8 are pictures), row 3 is near: nothing beyond it.
    expect(asked(backend)).toEqual(Array.from({ length: 10 }, (_, i) => `p${i}.jpg`));
  });

  it('drops the requests queued for what scrolled out of view', async () => {
    sized();
    const { answers, thumb_get } = held();
    const { backend } = await thumbnails({ thumb_get });
    await settle();
    expect(asked(backend)).toEqual(['p0.jpg', 'p1.jpg', 'p2.jpg']);

    viewport().scrollTop = 20 * TILE_HEIGHT;
    await fireEvent.scroll(viewport());
    await settle();
    answers.get('p0.jpg')?.resolve(picture(1, 1));
    await settle();

    expect(asked(backend).slice(0, 4)).toEqual(['p0.jpg', 'p1.jpg', 'p2.jpg', 'p58.jpg']); // row 20: items 60 to 62
    expect(asked(backend)).not.toContain('p3.jpg');
  });

  it('does not ask for pictures when the layout is not thumbnails, and stops when it changes', async () => {
    sized();
    const { answers, thumb_get } = held();
    const { backend, view } = await thumbnails({ thumb_get });
    await settle();
    expect(asked(backend)).toHaveLength(3);

    view.layout = 'detailed';
    await tick();
    for (const answer of answers.values()) {
      answer.resolve(picture(1, 1));
    }
    await settle();

    expect(asked(backend)).toHaveLength(3);
  });

  it('asks for the folder that holds a search result or a flat view item, from its path below the folder searched', async () => {
    sized();
    const backend = fakeBackend({ thumb_get: () => Promise.reject(refusal('unsupported')) });
    const tabs = new PaneTabs(backend.ipc, 'left');
    await tabs.view.listing.openResults(7);
    backend.last().emit({ kind: 'start', path: '/home/me', parent: '/home', crumbs: [], total: 3, results: 7 });
    backend.last().emit({
      kind: 'batch',
      offset: 0,
      entries: [entry('docs/deep/a.png'), entry('top.jpg'), entry('docs/b.txt')],
    });
    await mountThumbnails(backend, tabs);
    await settle();

    const requests = backend.invoke.mock.calls.filter(([name]) => name === 'thumb_get').map(([, args]) => args);
    expect(requests).toEqual([
      { path: '/home/me/docs/deep', name: 'a.png', size: 128 },
      { path: '/home/me', name: 'top.jpg', size: 128 },
    ]);
    expect(screen.getByText('docs/deep/a')).toBeInTheDocument(); // the tile is named by its path, as the row is
  });

  it('makes no request inside an archive: its pictures have their icons', async () => {
    sized();
    const backend = fakeBackend({ thumb_get: () => picture(1, 1) });
    const tabs = new PaneTabs(backend.ipc, 'left');
    await tabs.view.listing.openArchive({ archive: '/pics/a.zip', dir: '' });
    backend.last().emit({
      kind: 'start',
      path: '/pics',
      parent: '/',
      crumbs: [],
      total: 1,
      archive: { archive: '/pics/a.zip', dir: '' },
    });
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('inside.jpg')] });
    await mountThumbnails(backend, tabs);
    await settle();

    expect(asked(backend)).toEqual([]);
    expect(screen.getByText('inside')).toBeInTheDocument();
    expect(document.querySelector('canvas')).toBeNull();
    expect(document.querySelector('.row.tile .ic')).not.toBeNull();
  });
});

describe('Pane thumbnails: what a tile shows', () => {
  /** Stands in for the 2D context: records what is drawn. */
  function canvases() {
    const drawn: { width: number; height: number; data: number[] }[] = [];
    const original = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = (() => ({
      createImageData: (width: number, height: number) => ({ data: new Uint8ClampedArray(width * height * 4), width, height }),
      putImageData: (image: { data: Uint8ClampedArray; width: number; height: number }) =>
        drawn.push({ width: image.width, height: image.height, data: Array.from(image.data) }),
    })) as never;
    afterEach(() => (HTMLCanvasElement.prototype.getContext = original));

    return drawn;
  }

  it('draws the pixels the backend sent in place of the icon, and keeps the name under it', async () => {
    sized();
    const drawn = canvases();
    const pixels = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    await thumbnails({ thumb_get: () => picture(3, 1, pixels) }, [entry('a.jpg')]);
    await settle();

    const canvas = tileAt(1).querySelector('canvas') as HTMLCanvasElement;
    expect(canvas).not.toBeNull();
    expect([canvas.width, canvas.height]).toEqual([3, 1]);
    expect(canvas).toHaveAttribute('aria-hidden', 'true');
    expect(drawn).toEqual([{ width: 3, height: 1, data: pixels }]);
    expect(tileAt(1).querySelector('.ic')).toBeNull();
    expect(tileAt(1)).toHaveTextContent('a.jpg');
  });

  it('shows the icon until the picture comes, and again for a picture the backend could not read', async () => {
    sized();
    canvases();
    const first = pending();
    const answers = [first.answer, Promise.reject(refusal('invalidInput')), Promise.reject(refusal('unsupported'))];
    await thumbnails({ thumb_get: () => answers.shift() ?? null } as never, [entry('a.jpg'), entry('b.heic'), entry('c.png')]);

    expect(tileAt(1).querySelector('.ic')).not.toBeNull(); // asked, not answered
    await settle();
    expect(tileAt(2).querySelector('.ic')).not.toBeNull();
    expect(tileAt(3).querySelector('.ic')).not.toBeNull();
    expect(document.querySelector('canvas')).toBeNull();

    first.resolve(picture(1, 1));
    await settle();
    expect(tileAt(1).querySelector('canvas')).not.toBeNull();
    expect(tileAt(1).querySelector('.ic')).toBeNull();
    expect(tileAt(2).querySelector('canvas')).toBeNull();
  });

  it('shows the icon for a picture whose answer is not a picture', async () => {
    sized();
    canvases();
    await thumbnails({ thumb_get: () => new ArrayBuffer(3) }, [entry('a.jpg')]);
    await settle();

    expect(tileAt(1).querySelector('canvas')).toBeNull();
    expect(tileAt(1).querySelector('.ic')).not.toBeNull();
  });

  it('asks for a picture again when its file changed, and not when the folder is shown again', async () => {
    sized();
    canvases();
    const { backend, tabs } = await thumbnails({ thumb_get: () => picture(1, 1) }, [entry('a.jpg', 'file', { size: 1, modified: 5 })]);
    await settle();
    expect(asked(backend)).toEqual(['a.jpg']);

    // The folder is listed again with the same file: nothing is asked.
    await tabs.view.listing.reload();
    backend.last().emit({
      kind: 'start',
      path: '/big',
      parent: '/',
      crumbs: [],
      total: 1,
    });
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('a.jpg', 'file', { size: 1, modified: 5 })] });
    await settle();
    expect(asked(backend)).toEqual(['a.jpg']);

    // The file was written meanwhile.
    await tabs.view.listing.reload();
    backend.last().emit({ kind: 'start', path: '/big', parent: '/', crumbs: [], total: 1 });
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('a.jpg', 'file', { size: 2, modified: 9 })] });
    await settle();
    expect(asked(backend)).toEqual(['a.jpg', 'a.jpg']);
  });
});
