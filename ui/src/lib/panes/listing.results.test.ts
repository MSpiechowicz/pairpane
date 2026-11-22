import { describe, expect, it } from 'vitest';

import type { ListEvent } from '$lib/ipc/bindings/ListEvent';

import { entry, fakeBackend } from '../../../tests/panes';
import { Listing } from './listing.svelte';

const start = (total: number, results?: number, path = '/home/me'): ListEvent => ({
  kind: 'start',
  path,
  parent: '/home',
  crumbs: [{ name: 'me', path: '/home/me' }],
  total,
  results,
});

/** Delivers the results of search 7 on the latest channel: two rows, a file below a folder and a folder. */
function deliver(backend: ReturnType<typeof fakeBackend>) {
  backend.last().emit(start(2, 7));
  backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('docs/deep/a.txt'), entry('src/lib', 'dir')] });
  backend.last().emit({ kind: 'done', dirs: 1, files: 1, bytes: 0, free: null });
}

async function shown() {
  const backend = fakeBackend();
  const listing = new Listing(backend.ipc, 'left');
  await listing.openResults(7);
  deliver(backend);
  return { backend, listing };
}

describe('Listing of search results', () => {
  it('asks for the results of the search and lists the rows under their relative names', async () => {
    const { backend, listing } = await shown();

    expect(backend.invoke).toHaveBeenCalledWith('find_open', expect.objectContaining({ pane: 'left', id: 7 }));
    expect(listing.results).toBe(7);
    expect(listing.entries.map((row) => row?.name)).toEqual(['docs/deep/a.txt', 'src/lib']);
    expect(listing.folder).toBe('/home/me');
    expect(listing.parent).toBe('/home');
    expect(listing.status).toBe('ready');
  });

  it('has no path of one folder: its names are not plain names, and it is not read-only either', async () => {
    const { listing } = await shown();

    expect(listing.path).toBeUndefined();
    expect(listing.readonly).toBe(false);
  });

  it('is another place than the folder searched', async () => {
    const { backend, listing } = await shown();
    const results = listing.location;

    await listing.open('/home/me');
    backend.last().emit(start(0));

    expect(listing.results).toBeUndefined();
    expect(listing.path).toBe('/home/me');
    expect(listing.location).not.toBe(results);
    expect(listing.entries).toHaveLength(0); // not a reload: the rows of the results are not kept
  });

  it('lists the same results again on reload, keeping the rows until the new ones arrive', async () => {
    const { backend, listing } = await shown();

    await listing.reload();

    expect(backend.invoke).toHaveBeenLastCalledWith('find_open', expect.objectContaining({ id: 7 }));
    backend.last().emit(start(2, 7));
    expect(listing.entries.map((row) => row?.name)).toEqual(['docs/deep/a.txt', 'src/lib']);

    backend.last().emit({ kind: 'batch', offset: 0, entries: [entry('docs/deep/a.txt')] });
    expect(listing.results).toBe(7);
  });

  it('another search replaces the rows', async () => {
    const { backend, listing } = await shown();

    await listing.openResults(8);
    backend.last().emit(start(0, 8));

    expect(listing.results).toBe(8);
    expect(listing.entries).toHaveLength(0);
  });

  it('enter on a folder row opens the real folder below the folder searched', async () => {
    const { backend, listing } = await shown();

    await listing.enter('src/lib');

    expect(backend.invoke).toHaveBeenLastCalledWith('pane_open', expect.objectContaining({ path: '/home/me/src/lib', child: null }));
    backend.last().emit(start(0, undefined, '/home/me/src/lib'));
    expect(listing.results).toBeUndefined();
    expect(listing.path).toBe('/home/me/src/lib');
  });

  it('a search that is gone shows an error the pane keeps', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');

    await listing.openResults(9);
    backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'That search is gone.' } });

    expect(listing.status).toBe('error');
    expect(listing.error).toBe('That search is gone.');
  });
});
