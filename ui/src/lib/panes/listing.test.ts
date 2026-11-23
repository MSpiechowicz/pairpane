import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ListEvent } from '$lib/ipc/bindings/ListEvent';

import { formatDate, formatFree, formatSelection, formatSize, formatSummary, iconClass, splitName } from './format';
import { Listing, SPINNER_DELAY_MS } from './listing.svelte';
import { entry, fakeBackend } from '../../../tests/panes';

const start = (total: number, path = '/home/me'): ListEvent => ({
  kind: 'start',
  path,
  parent: '/home',
  crumbs: [{ name: '/', path: '/' }],
  total,
});

describe('Listing', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('sizes the list on start, fills batches in place and keeps the summary of done', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');

    await listing.open(null);
    expect(backend.last().args).toMatchObject({ pane: 'left', path: null, child: null, showHidden: false });

    backend.last().emit(start(4));
    expect(listing.entries).toHaveLength(4);
    expect(listing.entries[0]).toBeUndefined();

    backend.last().emit({ kind: 'batch', offset: 2, entries: [entry('c'), entry('d')] });
    expect(listing.entries.map((e) => e?.name)).toEqual([undefined, undefined, 'c', 'd']);
    expect(listing.firstScreenMs).toBeTypeOf('number');

    backend.last().emit({ kind: 'done', dirs: 1, files: 3, bytes: 10, free: null });
    expect(listing.status).toBe('ready');
    expect(listing.summary).toEqual({ dirs: 1, files: 3, bytes: 10, free: null });
    expect(listing.path).toBe('/home/me');
  });

  it('tells its hooks when a listing starts (another folder or a reload of this one) and is done, and keeps the free space', async () => {
    const backend = fakeBackend();
    const calls: string[] = [];
    const listing = new Listing(backend.ipc, 'left', {
      onStart: (reload) => calls.push(reload ? 'reload' : 'start'),
      onDone: () => calls.push('done'),
    });

    await listing.open('/a');
    backend.last().emit(start(0, '/a'));
    backend.last().emit({ kind: 'done', dirs: 0, files: 0, bytes: 0, free: 5000 });
    expect(listing.summary?.free).toBe(5000);

    await listing.reload();
    backend.last().emit(start(0, '/a'));
    backend.last().emit({ kind: 'done', dirs: 0, files: 0, bytes: 0, free: null });
    await listing.open('/b');
    backend.last().emit(start(0, '/b'));

    expect(calls).toEqual(['start', 'done', 'reload', 'done', 'start']);
  });

  it('counts the times the folder shown is listed again, and not the times another folder is', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');

    await listing.open('/a');
    backend.last().emit(start(0, '/a'));
    expect(listing.reloads).toBe(0);

    await listing.reload();
    backend.last().emit(start(0, '/a'));
    expect(listing.reloads).toBe(1);

    await listing.open('/b');
    backend.last().emit(start(0, '/b'));
    expect(listing.reloads).toBe(1);
  });

  it('drops events of an earlier open', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');

    await listing.open('/a');
    const earlier = backend.last();
    await listing.open('/b');
    earlier.emit(start(9, '/a'));

    expect(listing.path).toBeUndefined();
    expect(listing.entries).toHaveLength(0);
  });

  it('shows the spinner only when loading takes longer than the delay', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');

    await listing.open(null);
    vi.advanceTimersByTime(SPINNER_DELAY_MS - 1);
    expect(listing.slow).toBe(false);
    vi.advanceTimersByTime(1);
    expect(listing.slow).toBe(true);

    backend.last().emit(start(0));
    backend.last().emit({ kind: 'done', dirs: 0, files: 0, bytes: 0, free: null });
    expect(listing.slow).toBe(false);
  });

  it('a change on disk lists the folder again, after the running listing when it is still loading', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'right');

    await listing.open('/x');
    backend.last().emit(start(0, '/x'));
    backend.last().emit({ kind: 'changed' });
    expect(backend.opens).toHaveLength(1);

    backend.last().emit({ kind: 'done', dirs: 0, files: 0, bytes: 0, free: null });
    expect(backend.opens).toHaveLength(2);
    expect(backend.last().args).toMatchObject({ path: '/x', child: null });

    backend.last().emit(start(0, '/x'));
    backend.last().emit({ kind: 'done', dirs: 0, files: 0, bytes: 0, free: null });
    backend.last().emit({ kind: 'changed' });
    expect(backend.opens).toHaveLength(3);
  });

  it('enter opens a child of the current folder; hidden files are passed on', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.open('/x');
    backend.last().emit(start(0, '/x'));

    await listing.enter('sub');
    expect(backend.last().args).toMatchObject({ path: '/x', child: 'sub' });

    await listing.setShowHidden(true);
    expect(backend.last().args).toMatchObject({ path: '/x', showHidden: true });
  });

  it('lists a folder as the hook says about hidden entries, and the same setting again lists nothing', async () => {
    const backend = fakeBackend();
    let hidden = true;
    const listing = new Listing(backend.ipc, 'left', { showHidden: () => hidden });

    await listing.open('/x');
    expect(backend.last().args).toMatchObject({ path: '/x', showHidden: true });
    backend.last().emit(start(0, '/x'));

    await listing.setShowHidden(true);
    expect(backend.opens).toHaveLength(1);

    hidden = false;
    await listing.setShowHidden(false);
    expect(backend.opens).toHaveLength(2);
    expect(backend.last().args).toMatchObject({ path: '/x', showHidden: false });
  });

  it('onFail is called when the pane is left with nothing to show, not for a failed move away from a folder', async () => {
    const backend = fakeBackend();
    const onFail = vi.fn();
    const listing = new Listing(backend.ipc, 'left', { onFail });

    await listing.open('/gone');
    backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'This folder does not exist.' } });
    expect(onFail).toHaveBeenCalledTimes(1);

    await listing.open('/a');
    backend.last().emit(start(0, '/a'));
    await listing.open('/gone');
    backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'This folder does not exist.' } });
    expect(listing.notice).toBe('This folder does not exist.');
    expect(onFail).toHaveBeenCalledTimes(1);
  });

  it('an error event or a refused command shows the message and clears the entries', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.open('/x');
    backend.last().emit(start(1, '/x'));

    backend.last().emit({ kind: 'error', error: { code: 'io', message: 'You do not have permission.' } });
    expect(listing.status).toBe('error');
    expect(listing.error).toBe('You do not have permission.');
    expect(listing.entries).toHaveLength(0);

    vi.mocked(backend.ipc.invoke).mockRejectedValueOnce({ code: 'invalidInput', message: 'the path must be absolute' });
    await listing.open('relative');
    expect(listing.error).toBe('the path must be absolute');
  });
});

describe('Listing keeps the current folder in view', () => {
  it('a reload of the same folder keeps the old rows and summary until the new ones arrive', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.open('/x');
    backend.last().emit(start(2, '/x'));
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('a'), entry('b')] });
    backend.last().emit({ kind: 'done', dirs: 0, files: 2, bytes: 0, free: null });

    backend.last().emit({ kind: 'changed' });
    backend.last().emit(start(3, '/x'));

    expect(listing.entries.map((e) => e?.name)).toEqual(['a', 'b', undefined]);
    expect(listing.summary).toEqual({ dirs: 0, files: 2, bytes: 0, free: null });

    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('a'), entry('a2'), entry('b')] });
    expect(listing.entries.map((e) => e?.name)).toEqual(['a', 'a2', 'b']);
  });

  it('a folder that cannot be opened leaves the current one listed, with a notice', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.open('/x');
    backend.last().emit(start(1, '/x'));
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('locked', 'dir')] });
    backend.last().emit({ kind: 'done', dirs: 1, files: 0, bytes: 0, free: null });

    await listing.enter('locked');
    backend.last().emit({ kind: 'error', error: { code: 'io', message: 'You do not have permission to open this folder.' } });

    expect(listing.status).toBe('ready');
    expect(listing.error).toBeUndefined();
    expect(listing.notice).toBe('You do not have permission to open this folder.');
    expect(listing.path).toBe('/x');
    expect(listing.entries.map((e) => e?.name)).toEqual(['locked']);

    // The failed open dropped the backend's watch of /x: it is listed again, quietly (same notice, not loading).
    expect(backend.opens).toHaveLength(3);
    expect(backend.last().args).toMatchObject({ path: '/x', child: null });
    backend.last().emit(start(1, '/x'));
    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('locked', 'dir')] });
    backend.last().emit({ kind: 'done', dirs: 1, files: 0, bytes: 0, free: null });
    expect(listing.notice).toBe('You do not have permission to open this folder.');
    expect(listing.entries.map((e) => e?.name)).toEqual(['locked']);

    await listing.setShowHidden(true); // reloads /x, not home
    expect(backend.last().args).toMatchObject({ path: '/x', child: null });
    expect(listing.notice).toBeUndefined();
  });

  it('a failed reload of the shown folder does not try to list it again', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.open('/x');
    backend.last().emit(start(0, '/x'));
    backend.last().emit({ kind: 'done', dirs: 0, files: 0, bytes: 0, free: null });

    await listing.reload();
    backend.last().emit({ kind: 'error', error: { code: 'io', message: 'Gone.' } });

    expect(backend.opens).toHaveLength(2);
    expect(listing.notice).toBe('Gone.');
  });
});

describe('format', () => {
  it('sizes, dates and summaries read like the mockups', () => {
    expect(formatSize(98)).toBe('98 B');
    expect(formatSize(1331)).toBe('1.3 KB');
    expect(formatSize(47 * 1024)).toBe('47 KB');
    expect(formatSize(5.7 * 1024 ** 3)).toBe('5.7 GB');
    expect(formatDate(null)).toBe('');
    expect(formatDate(new Date(2026, 10, 20, 7, 5).getTime())).toBe('2026-11-20 07:05');
    expect(formatSummary(1, 2, 2048)).toBe('1 folder · 2 files · 2.0 KB');
    expect(formatSelection(1, 2, 2048)).toBe('Selected: 1 folder · 2 files · 2.0 KB');
    expect(formatFree(5.7 * 1024 ** 3)).toBe('5.7 GB free');
  });

  it('splits extensions off files only', () => {
    expect(splitName(entry('Cargo.toml'))).toEqual({ base: 'Cargo', ext: 'toml' });
    expect(splitName(entry('.bashrc'))).toEqual({ base: '.bashrc', ext: '' });
    expect(splitName(entry('trailing.'))).toEqual({ base: 'trailing.', ext: '' });
    expect(splitName(entry('v1.2', 'dir'))).toEqual({ base: 'v1.2', ext: '' });
    expect(iconClass(entry('photo.JPG'))).toBe('img');
    expect(iconClass(entry('src', 'dir'))).toBe('dir');
    expect(iconClass(entry('LICENSE'))).toBe('f');
  });
});
