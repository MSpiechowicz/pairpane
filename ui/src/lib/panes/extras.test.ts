import { describe, expect, it } from 'vitest';

import type { GitMark } from '$lib/ipc/bindings/GitMark';

import { archiveStart, arrive, entry, fakeBackend, resultsStart } from '../../../tests/panes';
import { extraTargetOf, gitCell, gitPlaceOf, NOTHING, severityOf, wantedExtras } from './extras';
import { Listing } from './listing.svelte';
import { PaneTabs } from './tabs.svelte';
import { THUMB_SIZE, targetOf } from './thumbs';

const BOTH = { dimensions: true, checksum: true, git: false };

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

  it('asks for the dimensions of an AVIF too, which has no thumbnail', async () => {
    const { listing } = await shown();

    expect(extraTargetOf(listing, entry('a.avif'))).toMatchObject({ picture: true });
    expect(extraTargetOf(listing, entry('b.AVIF'))).toMatchObject({ picture: true });
    expect(extraTargetOf(listing, entry('.avif'))).toMatchObject({ picture: false });
    expect(targetOf(listing, entry('a.avif'), THUMB_SIZE)).toBeUndefined();
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
    backend.last().emit(resultsStart('/home/me', 2, 7));

    expect(extraTargetOf(listing, entry('docs/deep/a.png'))).toMatchObject({ folder: '/home/me/docs/deep', name: 'a.png' });
    expect(extraTargetOf(listing, entry('top.bin'))).toMatchObject({ folder: '/home/me', name: 'top.bin' });
  });

  it('has none inside an archive', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.openArchive({ archive: '/pics/a.zip', dir: '' });
    backend.last().emit(archiveStart('/pics', 1, { archive: '/pics/a.zip', dir: '' }));

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

    expect(names(wantedExtras(view, rows, rows, { dimensions: true, checksum: false, git: false }).checksums)).toEqual([]);
    expect(names(wantedExtras(view, rows, rows, { dimensions: false, checksum: true, git: false }).dimensions)).toEqual([]);
    expect(wantedExtras(view, rows, rows, { dimensions: false, checksum: false, git: false })).toBe(NOTHING);
  });

  it('puts the rows in view first, then the others near them', async () => {
    const view = await shown([entry('a'), entry('b'), entry('c'), entry('d'), entry('e')]); // rows 1 to 5

    const wants = wantedExtras(view, { first: 3, last: 5 }, { first: 0, last: 6 }, BOTH);

    expect(names(wants.checksums)).toEqual(['c', 'd', 'a', 'b', 'e']);
  });
});

describe('wantedExtras: git', () => {
  const GIT = { dimensions: false, checksum: false, git: true };

  it('wants the folder of the items in view once, whatever their kinds (a folder has a status too), not for `..`', async () => {
    const view = await shown();

    const wants = wantedExtras(view, { first: 0, last: 5 }, { first: 0, last: 5 }, GIT);

    expect(wants.git).toEqual(['/pics']);
    expect(wants.dimensions).toEqual([]);
    expect(wants.checksums).toEqual([]);
    expect(wantedExtras(view, { first: 0, last: 0 }, { first: 0, last: 0 }, GIT).git).toEqual([]); // only `..`
  });

  it('wants no folder when the column is off', async () => {
    const view = await shown();
    const rows = { first: 0, last: 5 };

    expect(wantedExtras(view, rows, rows, BOTH).git).toEqual([]);
  });
});

describe('the Git place of an item', () => {
  it('has the real folder and name, says whether it is a folder, and is none for `..` or inside an archive', async () => {
    const { listing } = await shown();

    expect(gitPlaceOf(listing, entry('sub', 'dir'))).toEqual({ folder: '/pics', name: 'sub', dir: true });
    expect(gitPlaceOf(listing, entry('a.jpg'))).toEqual({ folder: '/pics', name: 'a.jpg', dir: false });
    expect(gitPlaceOf(listing, entry('..', 'dir'))).toBeUndefined();
    expect(gitPlaceOf(listing, undefined)).toBeUndefined();
  });

  it('is not a folder for a link, even one to a folder: git holds a link as one file', async () => {
    const { listing } = await shown();

    expect(gitPlaceOf(listing, entry('to-sub', 'dir', { linkTarget: 'sub' }))).toEqual({ folder: '/pics', name: 'to-sub', dir: false });
    expect(gitPlaceOf(listing, entry('to-file', 'file', { linkTarget: 'a.jpg' }))).toMatchObject({ dir: false });
    expect(gitPlaceOf(listing, entry('broken', 'other', { linkTarget: 'gone' }))).toMatchObject({ dir: false });
  });
});

describe('the Git cell', () => {
  const place = { folder: '/repo', name: 'a.txt' };

  it('is a mark, with its words, for an item git has something to say about', () => {
    expect(gitCell(place, { marks: new Map([['a.txt', 'staged']]) })).toMatchObject({ text: 'S', tone: 'ok' });
  });

  it('says "something inside" for the mark of a folder, and speaks of the file itself for a file', () => {
    const marks = new Map<string, GitMark>([['d', 'modified'], ['f', 'modified'], ['u', 'untracked'], ['s', 'staged'], ['c', 'conflicted']]);
    const tip = (name: string, dir: boolean) => gitCell({ folder: '/repo', name, dir }, { marks }).tip;

    expect(tip('d', true)).toBe('Modified: something inside has changes that are not staged');
    expect(tip('s', true)).toMatch(/^Staged: something inside/);
    expect(tip('c', true)).toMatch(/^Conflicted: something inside/);
    expect(tip('u', true)).toMatch(/^Untracked: .*what is inside it/);
    expect(tip('f', false)).toBe('Modified: has changes that are not staged');
    expect(tip('f', false)).not.toMatch(/inside/);
  });

  it('is empty for a clean item, a folder outside a repository, and what is not known', () => {
    expect(gitCell(place, { marks: new Map() })).toEqual({ text: '' });
    expect(gitCell(place, null)).toEqual({ text: '' });
    expect(gitCell(place, undefined)).toEqual({ text: '' });
    expect(gitCell(undefined, { marks: new Map([['a.txt', 'modified']]) })).toEqual({ text: '' });
  });

  it('is a dash with the reason for a repository too big to read', () => {
    expect(gitCell(place, { problem: 'too big' })).toEqual({ text: '–', tip: 'too big' });
  });

  it('goes by severity: conflicted, modified, staged, untracked, ignored, and none for the rest', () => {
    const folder = (mark: 'conflicted' | 'modified' | 'staged' | 'untracked' | 'ignored') => ({ marks: new Map([['a.txt', mark]]) });

    expect((['conflicted', 'modified', 'staged', 'untracked', 'ignored'] as const).map((mark) => severityOf(place, folder(mark)))).toEqual([0, 1, 2, 3, 4]);
    expect(severityOf(place, { marks: new Map() })).toBeUndefined();
    expect(severityOf(place, null)).toBeUndefined();
    expect(severityOf(place, { problem: 'x' })).toBeUndefined();
  });
});
