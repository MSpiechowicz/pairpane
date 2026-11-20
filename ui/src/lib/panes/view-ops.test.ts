import { describe, expect, it } from 'vitest';

import { arrive, deliver, entry, fakeBackend } from '../../../tests/panes';
import { PaneView } from './view.svelte';

/** A pane showing `/work` (rows: `..`, docs, a.txt, b.md, c.txt). */
async function shown() {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'left');
  const channel = await deliver(backend, () => view.listing.open('/work'), '/work', [
    entry('docs', 'dir'),
    entry('a.txt'),
    entry('b.md'),
    entry('c.txt'),
  ]);
  return { backend, view, channel };
}

const under = (view: PaneView) => view.entryAt(view.cursor)?.name;

describe('PaneView.deselect', () => {
  it('deselects the names a job took and leaves the cursor where it is', async () => {
    const { view } = await shown();
    view.selectAll();
    view.setCursor(3);

    view.deselect(view.selectedShown().map((entry) => entry.name));

    expect(view.selection).toBeUndefined();
    expect(view.isSelected('a.txt')).toBe(false);
    expect(under(view)).toBe('b.md');
  });

  it('keeps selected entries the quick filter hid, since the job never saw them', async () => {
    const { view } = await shown();
    view.selectAll();
    view.setFilter('.txt');
    const taken = view.selectedShown().map((entry) => entry.name);

    view.deselect(taken);
    view.setFilter('');

    expect(taken).not.toContain('b.md');
    expect(view.isSelected('b.md')).toBe(true);
    expect(taken.some((name) => view.isSelected(name))).toBe(false);
  });

  it('is harmless when nothing is selected', async () => {
    const { view } = await shown();

    view.deselect(['a.txt']);

    expect(view.selection).toBeUndefined();
  });
});

describe('PaneView selection under the quick filter', () => {
  it('counts and lists only the selected entries the filter shows; the hidden ones stay selected', async () => {
    const { view } = await shown();
    view.selectAll();
    const all = view.selection;

    view.setFilter('a.');
    expect(view.selectedShown().map((entry) => entry.name)).toEqual(['a.txt']);
    expect(view.selection).toMatchObject({ dirs: 0, files: 1 });

    view.setFilter('nothing matches');
    expect(view.selectedShown()).toEqual([]);
    expect(view.selection).toBeUndefined();

    view.setFilter('');
    expect(view.selection).toEqual(all);
  });
});

describe('PaneView.pointAt', () => {
  it('puts the cursor on a listed entry', async () => {
    const { view } = await shown();

    view.pointAt('c.txt');

    expect(under(view)).toBe('c.txt');
  });

  it('waits for an entry that is not listed yet (just renamed) and takes it when the listing has it', async () => {
    const { backend, view, channel } = await shown();
    view.setCursor(2); // a.txt, about to be renamed to z.txt

    view.pointAt('z.txt');
    expect(under(view)).toBe('a.txt');

    // The watch lists the folder again, now with the new name.
    channel.emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: 3 });
    channel.emit({ kind: 'batch', offset: 0, entries: [entry('docs', 'dir'), entry('b.md'), entry('z.txt')] });
    channel.emit({ kind: 'done', dirs: 1, files: 2, bytes: 0, free: null });

    expect(under(view)).toBe('z.txt');
    expect(backend.opens).toHaveLength(1);
  });

  it('gives up when the cursor is moved meanwhile, so a later listing does not pull it away', async () => {
    const { view, channel } = await shown();
    view.pointAt('z.txt');

    view.setCursor(3); // b.md
    channel.emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: 2 });
    channel.emit({ kind: 'batch', offset: 0, entries: [entry('b.md'), entry('z.txt')] });
    channel.emit({ kind: 'done', dirs: 0, files: 2, bytes: 0, free: null });

    expect(under(view)).toBe('b.md');
  });

  it('gives up when another folder is opened', async () => {
    const { backend, view } = await shown();
    view.pointAt('z.txt');

    await deliver(backend, () => view.listing.open('/other'), '/other', [entry('a'), entry('z.txt')]);

    expect(under(view)).toBeUndefined(); // the cursor is on `..`
    expect(view.isUp(view.cursor)).toBe(true);
  });

  it('is not confused by a listing that finishes before the entry is there', async () => {
    const { backend, view } = await shown();
    view.pointAt('z.txt');

    arrive(backend.last(), '/work', [entry('b.md'), entry('c.txt')]); // an older listing, without the new name
    expect(view.isUp(view.cursor)).toBe(true);

    arrive(backend.last(), '/work', [entry('b.md'), entry('z.txt')]);
    expect(under(view)).toBe('z.txt');
  });
});
