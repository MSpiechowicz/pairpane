import { describe, expect, it } from 'vitest';

import { arrive, entry, fakeBackend } from '../../../tests/panes';
import { extraTargetOf, NOTHING, wantedExtras } from './extras';
import { Listing } from './listing.svelte';
import { PaneTabs } from './tabs.svelte';

const BOTH = { dimensions: true, checksum: true };

/** A pane in `/pics` (so row 0 is `..`) with these entries. */
async function shown(entries = [entry('a.jpg'), entry('b.txt'), entry('sub', 'dir'), entry('c.PNG')]) {
  const backend = fakeBackend();
  const tabs = new PaneTabs(backend.ipc, 'left');
  await tabs.view.listing.open('/pics');
  arrive(backend.last(), '/pics', entries);

  return tabs.view;
}

describe('extraTargetOf', () => {
  it('has the folder and the name of a file, and says whether it looks like a picture', async () => {
    const { listing } = await shown();

    expect(extraTargetOf(listing, entry('a.jpg'))).toMatchObject({ folder: '/pics', name: 'a.jpg', picture: true });
    expect(extraTargetOf(listing, entry('b.txt'))).toMatchObject({ folder: '/pics', name: 'b.txt', picture: false });
    expect(extraTargetOf(listing, entry('c.HEIC'))).toMatchObject({ picture: true });
  });

  it('has nothing for a folder, a special file or a row that is not there (`..`)', async () => {
    const { listing } = await shown();

    expect(extraTargetOf(listing, entry('album.jpg', 'dir'))).toBeUndefined();
    expect(extraTargetOf(listing, entry('pipe', 'other'))).toBeUndefined();
    expect(extraTargetOf(listing, undefined)).toBeUndefined();
  });

  it('tells a changed file (its size, its time) and another folder from the same file', async () => {
    const { listing } = await shown();
    const key = (extra: Parameters<typeof entry>[2]) => extraTargetOf(listing, entry('a.jpg', 'file', extra))?.key;

    expect(new Set([key({}), key({ size: 5 }), key({ modified: 7 })]).size).toBe(3);
    expect(key({ size: 5, modified: 7 })).toBe(key({ size: 5, modified: 7 }));
  });

  it('has the folder that holds a search result, or a file of a flat view: the names are paths below the folder searched', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.openResults(7);
    backend.last().emit({ kind: 'start', path: '/home/me', parent: '/home', crumbs: [], total: 2, results: 7 });

    expect(extraTargetOf(listing, entry('docs/deep/a.png'))).toMatchObject({ folder: '/home/me/docs/deep', name: 'a.png' });
    expect(extraTargetOf(listing, entry('top.bin'))).toMatchObject({ folder: '/home/me', name: 'top.bin' });
  });

  it('has none inside an archive', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.openArchive({ archive: '/pics/a.zip', dir: '' });
    backend.last().emit({
      kind: 'start',
      path: '/pics',
      parent: '/',
      crumbs: [],
      total: 1,
      archive: { archive: '/pics/a.zip', dir: '' },
    });

    expect(listing.readonly).toBe(true);
    expect(extraTargetOf(listing, entry('a.jpg'))).toBeUndefined();
  });
});

describe('wantedExtras', () => {
  const names = (list: readonly { name: string }[]) => list.map((target) => target.name);

  it('wants the dimensions of the pictures and the checksums of the files, not of folders and `..`', async () => {
    const view = await shown();
    const rows = { first: 0, last: 5 }; // .., a.jpg, b.txt (sorted by the backend order), sub, c.PNG

    const wants = wantedExtras(view, rows, rows, BOTH);

    expect(names(wants.dimensions)).toEqual(['a.jpg', 'c.PNG']);
    expect(names(wants.checksums)).toEqual(['a.jpg', 'b.txt', 'c.PNG']);
  });

  it('wants only the columns that are shown, and nothing when none is', async () => {
    const view = await shown();
    const rows = { first: 0, last: 5 };

    expect(names(wantedExtras(view, rows, rows, { dimensions: true, checksum: false }).checksums)).toEqual([]);
    expect(names(wantedExtras(view, rows, rows, { dimensions: false, checksum: true }).dimensions)).toEqual([]);
    expect(wantedExtras(view, rows, rows, { dimensions: false, checksum: false })).toBe(NOTHING);
  });

  it('puts the rows in view first, then the others near them', async () => {
    const view = await shown([entry('a'), entry('b'), entry('c'), entry('d'), entry('e')]); // rows 1 to 5

    const wants = wantedExtras(view, { first: 3, last: 5 }, { first: 0, last: 6 }, BOTH);

    expect(names(wants.checksums)).toEqual(['c', 'd', 'a', 'b', 'e']);
  });
});
