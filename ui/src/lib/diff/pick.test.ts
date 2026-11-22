import { describe, expect, it } from 'vitest';

import { PaneTabs } from '$lib/panes/tabs.svelte';
import type { PaneView } from '$lib/panes/view.svelte';

import { arrive, entry, fakeBackend } from '../../../tests/panes';
import {
  ARCHIVE_NOTICE,
  FOLDER_NOTICE,
  LINK_NOTICE,
  NEED_TWO_NOTICE,
  OTHER_NOTICE,
  pickFiles,
  QUICK_NOTICE,
  SAME_NOTICE,
} from './pick';

/** Two panes, `/left` and `/right`, listed with the entries given. */
async function panes(left = [entry('a.txt'), entry('b.txt'), entry('c.txt'), entry('sub', 'dir')], right = [entry('a.txt'), entry('z.txt')]) {
  const backend = fakeBackend();
  const tabs = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  for (const [side, path, entries] of [
    ['left', '/left', left],
    ['right', '/right', right],
  ] as const) {
    await tabs[side].view.listing.open(path);
    arrive(backend.opens.at(-1)!, path, entries);
  }

  return { backend, left: tabs.left.view, right: tabs.right.view };
}

/** What Shift+F2 picks with the left pane active. */
const pick = (left: PaneView, right: PaneView, quick = false) => pickFiles({ active: left, other: right, side: 'left', quick });

describe('picking the two files of a comparison', () => {
  it('compares the file under the cursor of each pane, the left pane`s on the left', async () => {
    const { left, right } = await panes();
    left.pointAt('b.txt');
    right.pointAt('z.txt');

    expect(pick(left, right)).toEqual({ left: { path: '/left', name: 'b.txt' }, right: { path: '/right', name: 'z.txt' } });
  });

  it('keeps the sides when the right pane is the active one', async () => {
    const { left, right } = await panes();
    left.pointAt('b.txt');
    right.pointAt('z.txt');

    expect(pickFiles({ active: right, other: left, side: 'right', quick: false })).toEqual({
      left: { path: '/left', name: 'b.txt' },
      right: { path: '/right', name: 'z.txt' },
    });
  });

  it('compares the two selected files of the active pane, in the order of the folder', async () => {
    const { left, right } = await panes();
    left.select(['c.txt', 'a.txt']);

    expect(pick(left, right)).toEqual({ left: { path: '/left', name: 'a.txt' }, right: { path: '/left', name: 'c.txt' } });
  });

  it('needs exactly two when files are selected: one, and more than two, are refused', async () => {
    const { left, right } = await panes();

    left.select(['a.txt']);
    expect(pick(left, right)).toBe(NEED_TWO_NOTICE);
    left.select(['a.txt', 'b.txt', 'c.txt']);
    expect(pick(left, right)).toBe(NEED_TWO_NOTICE);
  });

  it('refuses a pane that has not listed anything yet, on either side', async () => {
    const { right } = await panes();
    const unlisted = new PaneTabs(fakeBackend().ipc, 'left').view;
    right.pointAt('a.txt');

    expect(pick(unlisted, right)).toBe(NEED_TWO_NOTICE);
    expect(pick(right, unlisted)).toBe(NEED_TWO_NOTICE);
  });

  it('refuses a folder and the parent row, with their own words', async () => {
    const { left, right } = await panes();

    left.pointAt('sub');
    expect(pick(left, right)).toBe(FOLDER_NOTICE);
    left.setCursor(0);
    expect(left.isUp(0)).toBe(true);
    expect(pick(left, right)).toBe(FOLDER_NOTICE);
    left.select(['a.txt', 'sub']);
    expect(pick(left, right)).toBe(FOLDER_NOTICE);
  });

  it('refuses a link and anything that is not a file', async () => {
    const { left, right } = await panes([entry('link', 'file', { linkTarget: 'a.txt' }), entry('pipe', 'other'), entry('a.txt')]);

    left.pointAt('link');
    expect(pick(left, right)).toBe(LINK_NOTICE);
    left.pointAt('pipe');
    expect(pick(left, right)).toBe(OTHER_NOTICE);
    left.select(['a.txt', 'link']);
    expect(pick(left, right)).toBe(LINK_NOTICE);
  });

  it('refuses the same file twice, when both panes show the same folder', async () => {
    const backend = fakeBackend();
    const tabs = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
    for (const side of ['left', 'right'] as const) {
      await tabs[side].view.listing.open('/same');
      arrive(backend.opens.at(-1)!, '/same', [entry('a.txt')]);
      tabs[side].view.pointAt('a.txt');
    }

    expect(pick(tabs.left.view, tabs.right.view)).toBe(SAME_NOTICE);
  });

  it('compares files of the same name in two folders', async () => {
    const { left, right } = await panes();
    left.pointAt('a.txt');
    right.pointAt('a.txt');

    expect(pick(left, right)).toEqual({ left: { path: '/left', name: 'a.txt' }, right: { path: '/right', name: 'a.txt' } });
  });

  it('refuses the cursor rule while the quick view hides the other pane, but not a selection', async () => {
    const { left, right } = await panes();
    left.pointAt('a.txt');

    expect(pick(left, right, true)).toBe(QUICK_NOTICE);
    left.select(['a.txt', 'b.txt']);
    expect(pick(left, right, true)).toEqual({ left: { path: '/left', name: 'a.txt' }, right: { path: '/left', name: 'b.txt' } });
  });

  it('resolves search results to the folder that holds each file', async () => {
    const { backend, left, right } = await panes();
    await left.listing.openResults(1);
    const channel = backend.opens.at(-1)!;
    channel.emit({ kind: 'start', path: '/left', parent: '/', crumbs: [], total: 2, results: 1 });
    channel.emit({ kind: 'batch', offset: 0, entries: [entry('docs/deep/a.txt'), entry('b.txt')] });
    channel.emit({ kind: 'done', dirs: 0, files: 2, bytes: 0, free: null });

    left.select(['docs/deep/a.txt', 'b.txt']);

    expect(pick(left, right)).toEqual({ left: { path: '/left/docs/deep', name: 'a.txt' }, right: { path: '/left', name: 'b.txt' } });
  });

  it('refuses a pane inside an archive, whether it is the active one or the other', async () => {
    const { backend, left, right } = await panes();
    await right.listing.openArchive({ archive: '/right/a.zip', dir: '' });
    const channel = backend.opens.at(-1)!;
    channel.emit({
      kind: 'start',
      path: '/right',
      parent: '/',
      crumbs: [],
      total: 1,
      archive: { archive: '/right/a.zip', dir: '' },
    });
    channel.emit({ kind: 'batch', offset: 0, entries: [entry('inner.txt')] });
    channel.emit({ kind: 'done', dirs: 0, files: 1, bytes: 0, free: null });
    left.pointAt('a.txt');
    right.pointAt('inner.txt');

    expect(pick(left, right)).toBe(ARCHIVE_NOTICE);
    expect(pickFiles({ active: right, other: left, side: 'right', quick: false })).toBe(ARCHIVE_NOTICE);
    right.select(['inner.txt']);
    expect(pickFiles({ active: right, other: left, side: 'right', quick: false })).toBe(ARCHIVE_NOTICE);
  });
});
