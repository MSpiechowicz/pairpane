import { describe, expect, it, vi } from 'vitest';

import { deliver, entry, fakeBackend } from '../../../tests/panes';
import { BUSY_RETRY_MS } from './sizes.svelte';
import { PaneView } from './view.svelte';

const sample = () => [
  entry('docs', 'dir'),
  entry('src', 'dir'),
  entry('a.txt', 'file', { size: 30, modified: 300 }),
  entry('b.md', 'file', { size: 10, modified: 100 }),
  entry('c.TXT', 'file', { size: 20, modified: 200 }),
];

/** A pane showing `/work` (rows: `..`, docs, src, a.txt, b.md, c.TXT). */
async function shown(entries = sample()) {
  const backend = fakeBackend();
  const view = new PaneView(backend.ipc, 'left');
  const channel = await deliver(backend, () => view.listing.open('/work'), '/work', entries);
  return { backend, view, channel };
}

const names = (view: PaneView) => Array.from({ length: view.count }, (_, row) => view.entryAt(row)?.name ?? '..');

describe('PaneView rows and cursor', () => {
  it('shows `..` first, then the entries; the cursor is clamped', async () => {
    const { view } = await shown();

    expect(names(view)).toEqual(['..', 'docs', 'src', 'a.txt', 'b.md', 'c.TXT']);
    expect(view.cursor).toBe(0);

    view.moveCursor(-1);
    expect(view.cursor).toBe(0);
    view.moveCursor(99);
    expect(view.cursor).toBe(5);
    view.setCursor(2);
    expect(view.cursor).toBe(2);
  });

  it('keeps the cursor on its entry when the sort changes', async () => {
    const { view } = await shown();
    view.setCursor(4); // b.md

    view.sortBy('size');
    expect(names(view)).toEqual(['..', 'docs', 'src', 'b.md', 'c.TXT', 'a.txt']);
    expect(view.entryAt(view.cursor)?.name).toBe('b.md');

    view.sortBy('size');
    expect(view.desc).toBe(true);
    expect(names(view)).toEqual(['..', 'docs', 'src', 'a.txt', 'c.TXT', 'b.md']);
    expect(view.entryAt(view.cursor)?.name).toBe('b.md');

    view.sortBy('ext'); // another column starts ascending
    expect(view.desc).toBe(false);
    expect(names(view).slice(3)).toEqual(['b.md', 'a.txt', 'c.TXT']);
  });

  it('keeps the sort across folders', async () => {
    const { backend, view } = await shown();
    view.sortBy('modified');
    view.sortBy('modified');

    await deliver(backend, () => view.listing.open('/other'), '/other', [entry('x', 'file', { modified: 1 }), entry('y', 'file', { modified: 2 })]);

    expect(view.column).toBe('modified');
    expect(view.desc).toBe(true);
    expect(names(view)).toEqual(['..', 'y', 'x']);
  });

  it('opens a folder or `..`, and does nothing for a file', async () => {
    const { backend, view } = await shown();

    await view.open(3);
    expect(backend.opens).toHaveLength(1);

    await view.open(2);
    expect(backend.last().args).toMatchObject({ path: '/work', child: 'src' });

    await view.open(0);
    expect(backend.last().args).toMatchObject({ path: '/', child: null });
  });

  it('going up puts the cursor on the folder we came from once its batch arrives', async () => {
    const backend = fakeBackend();
    const view = new PaneView(backend.ipc, 'left');
    await deliver(backend, () => view.listing.open('/home/me/work'), '/home/me/work', sample());

    await view.goUp();
    const channel = backend.last();
    channel.emit({ kind: 'start', path: '/home/me', parent: '/home', crumbs: [{ name: 'me', path: '/home/me' }], total: 3 });
    expect(view.cursor).toBe(0);

    channel.emit({ kind: 'batch', offset: 0, entries: [entry('apps', 'dir'), entry('notes.txt'), entry('work', 'dir')] });
    expect(view.entryAt(view.cursor)?.name).toBe('work');
  });
});

describe('PaneView going up and failed opens', () => {
  it('a later open of another folder does not inherit the cursor of the going up it replaced', async () => {
    const backend = fakeBackend();
    const view = new PaneView(backend.ipc, 'left');
    await deliver(backend, () => view.listing.open('/home/me/work'), '/home/me/work', sample());

    await view.goUp();
    await view.listing.open('/elsewhere'); // supersedes the going up before anything was listed
    backend.last().emit({ kind: 'start', path: '/elsewhere', parent: '/', crumbs: [], total: 2 });
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('work', 'dir'), entry('x', 'dir')] });

    expect(view.cursor).toBe(0);
  });

  it('a folder that cannot be opened keeps the selection, filter, sizes and cursor of the shown one', async () => {
    const { backend, view } = await shown();
    view.setCursor(1);
    view.space(); // docs: selected and measured
    view.setCursor(3);
    view.insert(); // a.txt
    view.setFilter('s');
    const cursor = view.cursor;

    await view.open(2); // src: fails
    backend.last().emit({ kind: 'error', error: { code: 'io', message: 'No permission.' } });

    // The shown folder is listed again quietly; that is a reload, not a new folder.
    expect(backend.last().args).toMatchObject({ path: '/work', child: null });
    backend.last().emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: 5 });
    backend.last().emit({ kind: 'batch', offset: 0, entries: sample() });
    backend.last().emit({ kind: 'done', dirs: 2, files: 3, bytes: 60, free: null });

    expect(view.listing.notice).toBe('No permission.');
    expect(view.isSelected('docs')).toBe(true);
    expect(view.isSelected('a.txt')).toBe(true);
    expect(view.filter).toBe('s');
    expect(view.cursor).toBe(cursor);
  });

  it('a running size job survives a failed open, and sizing still works afterwards', async () => {
    const { backend, view } = await shown();
    view.setCursor(1);
    view.space(); // docs: measuring
    expect(backend.sizes).toHaveLength(1);

    await view.open(2); // src fails
    backend.last().emit({ kind: 'error', error: { code: 'io', message: 'No permission.' } });
    backend.last().emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: 5 });

    backend.sizes[0].emit({ kind: 'progress', bytes: 10 });
    backend.sizes[0].emit({ kind: 'done', bytes: 4096 });
    expect(view.sizes.text(view.entryAt(1)!)).toBe('4.0 KB');
    expect(view.isSelected('docs')).toBe(true);

    view.setCursor(2);
    view.space(); // src
    expect(backend.sizes).toHaveLength(2);
    expect(backend.sizes[1].args).toMatchObject({ pane: 'left', path: '/work', name: 'src' });
    backend.sizes[1].emit({ kind: 'done', bytes: 5 });
    expect(view.sizes.text(view.entryAt(2)!)).toBe('5 B');
  });
});

describe('PaneView quick filter', () => {
  it('shows entries containing the text, keeps `..`, and moves the cursor to the first match', async () => {
    const { view } = await shown();
    view.setCursor(2); // src

    view.setFilter('TXT');
    expect(names(view)).toEqual(['..', 'a.txt', 'c.TXT']);
    expect(view.matches).toBe(2);
    expect(view.entryAt(view.cursor)?.name).toBe('a.txt');

    view.setFilter('c.');
    expect(view.entryAt(view.cursor)?.name).toBe('c.TXT');

    view.setFilter('');
    expect(view.count).toBe(6);
    expect(view.entryAt(view.cursor)?.name).toBe('c.TXT'); // stays where it is
  });

  it('moves the cursor off `..` to the first match', async () => {
    const { view } = await shown();
    expect(view.cursor).toBe(0);

    view.setFilter('s');
    expect(view.entryAt(view.cursor)?.name).toBe('docs');

    view.setCursor(0);
    view.setFilter('sr');
    expect(view.entryAt(view.cursor)?.name).toBe('src');

    view.setCursor(0);
    view.setFilter('nothing like it'); // no match: the cursor stays on `..`
    expect(view.isUp(view.cursor)).toBe(true);
  });

  it('clears when another folder opens but not when the same one reloads', async () => {
    const { backend, view, channel } = await shown();
    view.setFilter('txt');

    channel.emit({ kind: 'changed' });
    backend.last().emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: 5 });
    expect(view.filter).toBe('txt');

    await deliver(backend, () => view.listing.open('/other'), '/other', [entry('x')]);
    expect(view.filter).toBe('');
  });
});

describe('PaneView selection', () => {
  it('Space toggles the entry and moves down; `..` is never selected', async () => {
    const { view } = await shown();

    view.space(); // on `..`
    expect(view.selection).toBeUndefined();
    expect(view.cursor).toBe(1);

    view.setCursor(3);
    view.space();
    view.space(); // b.md
    expect(view.isSelected('a.txt')).toBe(true);
    expect(view.isSelected('b.md')).toBe(true);
    expect(view.cursor).toBe(5);

    view.setCursor(3);
    view.space();
    expect(view.isSelected('a.txt')).toBe(false);
  });

  it('sums the selection, computed folder sizes included', async () => {
    const { backend, view } = await shown();

    view.setCursor(1);
    view.space(); // docs: selected and measured
    view.setCursor(3);
    view.insert(); // a.txt
    expect(view.selection).toEqual({ dirs: 1, files: 1, bytes: 30 });

    backend.sizes[0].emit({ kind: 'done', bytes: 1000 });
    expect(view.selection).toEqual({ dirs: 1, files: 1, bytes: 1030 });
  });

  it('Insert selects without measuring a folder', async () => {
    const { backend, view } = await shown();

    view.setCursor(1);
    view.insert();

    expect(view.isSelected('docs')).toBe(true);
    expect(backend.sizes).toHaveLength(0);
  });

  it('survives sorting and a reload; names that vanish are dropped when the listing is done', async () => {
    const { backend, view, channel } = await shown();
    view.setCursor(3);
    view.insert();
    view.insert(); // a.txt, b.md

    view.sortBy('size');
    expect(view.selection).toEqual({ dirs: 0, files: 2, bytes: 40 });

    channel.emit({ kind: 'changed' });
    const reload = backend.last();
    reload.emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: 3 });
    reload.emit({ kind: 'batch', offset: 0, entries: [entry('a.txt', 'file', { size: 30 }), entry('c.TXT'), entry('z')] });
    expect(view.selection).toEqual({ dirs: 0, files: 1, bytes: 30 }); // b.md vanished and stops counting

    reload.emit({ kind: 'done', dirs: 0, files: 3, bytes: 30, free: null });
    expect(view.isSelected('b.md')).toBe(false);
    expect(view.isSelected('a.txt')).toBe(true);
  });

  it('clears when another folder opens', async () => {
    const { backend, view } = await shown();
    view.setCursor(3);
    view.insert();

    await deliver(backend, () => view.listing.open('/other'), '/other', [entry('a.txt')]);

    expect(view.selection).toBeUndefined();
    expect(view.isSelected('a.txt')).toBe(false);
  });

  it('`*` inverts the visible files, Select-all takes folders too, and both only touch what the filter shows', async () => {
    const { view } = await shown();
    view.setCursor(3);
    view.insert(); // a.txt

    view.invert();
    expect(view.selection).toEqual({ dirs: 0, files: 2, bytes: 30 }); // b.md, c.TXT

    view.selectAll();
    expect(view.selection).toEqual({ dirs: 2, files: 3, bytes: 60 });

    view.setFilter('md');
    view.invert();
    expect(view.isSelected('b.md')).toBe(false);
    expect(view.isSelected('a.txt')).toBe(true);
  });

  it('a mask selects or deselects the visible files that match, ignoring case and folders', async () => {
    const { view } = await shown();

    view.beginMask('select');
    expect(view.maskText).toBe('*');
    view.maskText = '*.txt;b*';
    view.applyMask();
    expect(view.maskMode).toBeUndefined();
    expect(view.selection).toEqual({ dirs: 0, files: 3, bytes: 60 });

    view.beginMask('deselect');
    view.maskText = '*.md';
    view.applyMask();
    expect(view.isSelected('b.md')).toBe(false);
    expect(view.selection?.files).toBe(2);

    view.beginMask('select');
    view.maskText = '*.*';
    view.applyMask();
    expect(view.selection).toEqual({ dirs: 0, files: 3, bytes: 60 }); // `*.*` is every file, never a folder

    view.setFilter('a.');
    view.beginMask('deselect');
    view.applyMask();
    expect(view.isSelected('a.txt')).toBe(false);
    expect(view.isSelected('c.TXT')).toBe(true); // hidden by the filter

    view.beginMask('select');
    view.cancelMask();
    expect(view.maskMode).toBeUndefined();
  });

  it('clicks: the cursor, a toggle with Ctrl, a range with Shift', async () => {
    const { view } = await shown();

    view.pick(2, 'cursor');
    expect(view.cursor).toBe(2);
    expect(view.selection).toBeUndefined();

    view.pick(4, 'toggle');
    expect(view.cursor).toBe(4);
    expect(view.isSelected('b.md')).toBe(true);

    view.pick(5, 'range');
    expect(view.selection).toEqual({ dirs: 0, files: 2, bytes: 30 }); // b.md, c.TXT

    view.pick(2, 'range'); // upwards from the cursor, folders included
    expect(view.selection).toEqual({ dirs: 1, files: 3, bytes: 60 });
  });
});

describe('PaneView folder sizes', () => {
  it('measures one folder at a time, in the order asked, showing progress and then the final size', async () => {
    const { view, backend } = await shown();

    view.setCursor(1);
    view.space(); // docs
    view.space(); // src
    expect(backend.sizes).toHaveLength(1);
    expect(backend.sizes[0].args).toMatchObject({ pane: 'left', path: '/work', name: 'docs' });
    expect(view.sizes.text(view.entryAt(1)!)).toBe('0 B');
    expect(view.sizes.text(view.entryAt(2)!)).toBeUndefined();

    backend.sizes[0].emit({ kind: 'progress', bytes: 2048 });
    expect(view.sizes.text(view.entryAt(1)!)).toBe('2.0 KB');

    backend.sizes[0].emit({ kind: 'done', bytes: 4096 });
    expect(view.sizes.text(view.entryAt(1)!)).toBe('4.0 KB');
    expect(backend.sizes).toHaveLength(2);
    expect(backend.sizes[1].args).toMatchObject({ name: 'src' });

    backend.sizes[1].emit({ kind: 'error', error: { code: 'io', message: 'x' } });
    expect(view.sizes.text(view.entryAt(2)!)).toBeUndefined();

    view.setCursor(1);
    view.space(); // known already: not measured again
    expect(backend.sizes).toHaveLength(2);
  });

  it('a job the backend cancelled ends, shows no size, and the next folder is measured', async () => {
    const { view, backend } = await shown();

    view.setCursor(1);
    view.space(); // docs
    view.space(); // src
    backend.sizes[0].emit({ kind: 'progress', bytes: 2048 });
    backend.sizes[0].emit({ kind: 'cancelled' });

    expect(view.sizes.text(view.entryAt(1)!)).toBeUndefined();
    expect(backend.sizes).toHaveLength(2);
    expect(backend.sizes[1].args).toMatchObject({ name: 'src' });
  });

  it('sorting by size uses the computed sizes', async () => {
    const { view, backend } = await shown();
    view.setCursor(1);
    view.space();
    backend.sizes[0].emit({ kind: 'done', bytes: 50 });
    view.space();
    backend.sizes[1].emit({ kind: 'done', bytes: 10 });

    view.sortBy('size');

    expect(names(view).slice(1, 3)).toEqual(['src', 'docs']);
  });

  it('a reload of the same folder keeps the sizes and the running job; another folder drops them', async () => {
    const { view, backend, channel } = await shown();
    view.setCursor(1);
    view.space();

    channel.emit({ kind: 'changed' });
    backend.last().emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: 5 });
    backend.sizes[0].emit({ kind: 'done', bytes: 7 });
    expect(view.sizes.text(sample()[0])).toBe('7 B');

    view.setCursor(2);
    view.space(); // src: queued behind nothing, now running
    const stale = backend.sizes[1];

    await deliver(backend, () => view.listing.open('/other'), '/other', [entry('src', 'dir'), entry('docs', 'dir')]);
    stale.emit({ kind: 'done', bytes: 99 }); // from the folder we left

    expect(view.sizes.text(entry('src', 'dir'))).toBeUndefined();
    expect(view.sizes.text(entry('docs', 'dir'))).toBeUndefined();
  });

  it('a reload forgets the sizes of folders that are gone once it is done', async () => {
    const { view, backend, channel } = await shown();
    view.setCursor(1);
    view.space(); // docs
    backend.sizes[0].emit({ kind: 'done', bytes: 7 });
    view.space(); // src
    backend.sizes[1].emit({ kind: 'done', bytes: 9 });

    channel.emit({ kind: 'changed' });
    const reload = backend.last();
    reload.emit({ kind: 'start', path: '/work', parent: '/', crumbs: [], total: 2 });
    reload.emit({ kind: 'batch', offset: 0, entries: [entry('src', 'dir'), entry('a.txt')] });
    expect(view.sizes.sizeOf('docs')).toBe(7); // not before the listing is done

    reload.emit({ kind: 'done', dirs: 1, files: 1, bytes: 0, free: null });
    expect(view.sizes.sizeOf('docs')).toBeUndefined();
    expect(view.sizes.sizeOf('src')).toBe(9);
  });

  it('a refused command ends the job and the next one starts', async () => {
    const { view, backend } = await shown();
    const invoke = backend.ipc.invoke as unknown as { mockRejectedValueOnce: (error: unknown) => void };
    invoke.mockRejectedValueOnce({ code: 'invalidInput', message: 'not the pane folder' });

    view.setCursor(1);
    view.space(); // docs: refused
    view.space(); // src
    await Promise.resolve();
    await Promise.resolve();

    expect(backend.sizes.map((job) => job.args.name)).toEqual(['src']);
  });

  it('a job refused as busy is asked again, and not after the pane moved on', async () => {
    vi.useFakeTimers();
    try {
      const { view, backend } = await shown();
      const invoke = backend.ipc.invoke as unknown as { mockRejectedValueOnce: (error: unknown) => void };
      invoke.mockRejectedValueOnce({ code: 'busy', message: 'the previous size calculation has not stopped yet' });

      view.setCursor(1);
      view.space(); // docs: busy
      view.space(); // src waits behind it
      await vi.advanceTimersByTimeAsync(0);
      expect(backend.sizes).toHaveLength(0);

      await vi.advanceTimersByTimeAsync(BUSY_RETRY_MS);
      expect(backend.sizes.map((job) => job.args.name)).toEqual(['docs']);

      invoke.mockRejectedValueOnce({ code: 'busy', message: 'still busy' });
      backend.sizes[0].emit({ kind: 'done', bytes: 1 }); // src is asked next and refused as busy
      view.sizes.clear(); // the pane opened another folder
      await vi.advanceTimersByTimeAsync(BUSY_RETRY_MS * 2);
      expect(backend.sizes).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
