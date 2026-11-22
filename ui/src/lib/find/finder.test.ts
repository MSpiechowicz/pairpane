import { describe, expect, it, vi } from 'vitest';

import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';
import { IpcFailure } from '$lib/ipc/transport';

import { done, findBackend, hit } from '../../../tests/find';
import { Finder } from './finder.svelte';
import { blankForm, toCriteria } from './query';

const criteria = toCriteria(blankForm(), false) as FindCriteria;

function setup(answers: Parameters<typeof findBackend>[0] = {}, shown: (id: number) => boolean = () => false) {
  const backend = findBackend(answers);
  const finder = new Finder(backend.ipc, shown);
  finder.show('/home/ann');
  return { ...backend, finder };
}

/** Starts a search and lets its answer (the id) arrive. */
async function started(s: ReturnType<typeof setup>) {
  await s.finder.start(criteria);
  return s.searches.at(-1)!;
}

describe('Finder', () => {
  it('is open on a folder, which a second show does not change', () => {
    const { finder } = setup();
    finder.show('/elsewhere');

    expect(finder.root).toBe('/home/ann');
    expect(finder.modal).toBe(true);
  });

  it('asks for the criteria in the folder and lists the results as they stream in', async () => {
    const s = setup();
    const search = await started(s);

    expect(search.args.query).toEqual({ root: '/home/ann', criteria });
    expect(s.finder.status).toBe('running');

    search.emit({ kind: 'found', hits: [hit('a.txt'), hit('b/c.txt')] });
    search.emit({ kind: 'progress', folders: 2, files: 9, bytes: 4096 });
    search.emit({ kind: 'found', hits: [hit('d.txt')] });

    expect(s.finder.rows.map((row) => row.kind === 'hit' && row.hit.path)).toEqual(['a.txt', 'b/c.txt', 'd.txt']);
    expect(s.finder.hits).toBe(3);
    expect(s.finder.progress).toEqual({ folders: 2, files: 9, bytes: 4096 });
  });

  it('events that arrive before the id does (the backend starts at once) are listed all the same', async () => {
    const s = setup({ find_start: () => new Promise(() => {}) });
    void s.finder.start(criteria);
    await vi.waitFor(() => expect(s.searches).toHaveLength(1));

    s.searches[0].emit({ kind: 'found', hits: [hit('early.txt')] });

    expect(s.finder.hits).toBe(1);
  });

  it('shows the groups of duplicates as a head and its files', async () => {
    const s = setup();
    const search = await started(s);

    search.emit({ kind: 'group', size: 100, hits: [hit('a'), hit('b')] });
    search.emit({ kind: 'group', size: 5, hits: [hit('c'), hit('d'), hit('e')] });

    expect(s.finder.rows.map((row) => (row.kind === 'group' ? `${row.count}x${row.size}` : row.hit.path))).toEqual([
      '2x100',
      'a',
      'b',
      '3x5',
      'c',
      'd',
      'e',
    ]);
    expect(s.finder.hits).toBe(5);
  });

  it('the done event gives the summary and ends the run', async () => {
    const s = setup();
    const search = await started(s);

    search.emit(done(7, { skipped: 2, truncated: true }));

    expect(s.finder.status).toBe('done');
    expect(s.finder.summary).toEqual({ found: 7, skipped: 2, truncated: true, cancelled: false });
  });

  it('an error event ends the run with its message', async () => {
    const s = setup();
    const search = await started(s);

    search.emit({ kind: 'error', error: { code: 'forbidden', message: 'Cannot search this folder' } });

    expect(s.finder.status).toBe('failed');
    expect(s.finder.failure).toBe('Cannot search this folder');
  });

  it('a search the backend refuses to start shows why and can be started again', async () => {
    const s = setup({
      find_start: () => {
        throw new IpcFailure({ code: 'busy', message: 'two searches are running; stop one or wait for it' });
      },
    });

    await s.finder.start(criteria);

    expect(s.finder.status).toBe('idle');
    expect(s.finder.failure).toBe('two searches are running; stop one or wait for it');
  });

  it('Stop cancels the search by its id and leaves the results; the done event says it was stopped', async () => {
    const s = setup();
    const search = await started(s);
    search.emit({ kind: 'found', hits: [hit('a')] });

    s.finder.stop();
    expect(s.finder.stopping).toBe(true);
    expect(s.calls('find_cancel')).toEqual([{ id: 1 }]);

    search.emit(done(1, { cancelled: true }));
    expect(s.finder.stopping).toBe(false);
    expect(s.finder.summary?.cancelled).toBe(true);
    expect(s.finder.hits).toBe(1);
  });

  it('a Stop before the id is known cancels as soon as it is', async () => {
    let answer: (id: number) => void = () => {};
    const s = setup({ find_start: () => new Promise((resolve) => (answer = resolve as never)) });
    const starting = s.finder.start(criteria);

    s.finder.stop();
    expect(s.calls('find_cancel')).toEqual([]);
    answer(9);
    await starting;

    expect(s.calls('find_cancel')).toEqual([{ id: 9 }]);
  });

  it('closing forgets the search, ignores what still arrives, and drops the results', async () => {
    const s = setup();
    const search = await started(s);
    search.emit({ kind: 'found', hits: [hit('a')] });

    s.finder.close();
    search.emit({ kind: 'found', hits: [hit('late')] });

    expect(s.calls('find_forget')).toEqual([{ id: 1 }]);
    expect(s.finder.modal).toBe(false);
    expect(s.finder.rows).toHaveLength(0);
    expect(s.finder.hits).toBe(0);
  });

  it('a search that was started and closed before its id came is forgotten when it comes', async () => {
    let answer: (id: number) => void = () => {};
    const s = setup({ find_start: () => new Promise((resolve) => (answer = resolve as never)) });
    const starting = s.finder.start(criteria);

    s.finder.close();
    answer(4);
    await starting;

    expect(s.calls('find_forget')).toEqual([{ id: 4 }]);
  });

  it('starting another search forgets the one before', async () => {
    const s = setup();
    const first = await started(s);
    first.emit(done(0));

    await s.finder.start(criteria);

    expect(s.calls('find_forget')).toEqual([{ id: 1 }]);
    expect(s.finder.hits).toBe(0);
    expect(s.finder.summary).toBeUndefined();
  });

  it('keeps the fields when it is closed and opened again, but not the results', async () => {
    const s = setup();
    s.finder.form.names = '*.rs';
    const search = await started(s);
    search.emit({ kind: 'found', hits: [hit('a.rs')] });
    s.finder.close();

    s.finder.show('/other');

    expect(s.finder.form.names).toBe('*.rs');
    expect(s.finder.rows).toHaveLength(0);
  });

  describe('feeding a pane', () => {
    it('lists the results of a search that is over, and closing then leaves it to the pane that shows it', async () => {
      const s = setup({}, () => true);
      const search = await started(s);
      search.emit({ kind: 'found', hits: [hit('a')] });
      expect(s.finder.feedable).toBe(false); // still running
      search.emit(done(1));
      const target = { openResults: vi.fn(async () => {}) };

      expect(s.finder.feedable).toBe(true);
      expect(await s.finder.feed(target)).toBe(true);
      s.finder.close();

      expect(target.openResults).toHaveBeenCalledWith(1);
      expect(s.calls('find_forget')).toEqual([]);
    });

    it('nothing is fed when there are only results inside archives, or none', async () => {
      const s = setup();
      const search = await started(s);
      search.emit({ kind: 'found', hits: [hit('a.zip', { inner: 'x/y.txt' })] });
      search.emit(done(1));

      expect(s.finder.inside).toBe(1);
      expect(s.finder.feedable).toBe(false);
      expect(await s.finder.feed({ openResults: vi.fn() })).toBe(false);
    });

    it('a pane that shows a search keeps it; once no pane does, the next close or start forgets it', async () => {
      const showing = new Set([1]);
      const s = setup({}, (id) => showing.has(id));
      const first = await started(s);
      first.emit({ kind: 'found', hits: [hit('a')] });
      first.emit(done(1));
      await s.finder.feed({ openResults: async () => {} });
      s.finder.close();

      s.finder.show('/home/ann');
      await s.finder.start(criteria);
      expect(s.calls('find_forget')).toEqual([]);

      showing.clear();
      s.finder.close();

      expect(s.calls('find_forget').map((args) => (args as { id: number }).id)).toEqual([2, 1]);
    });

    it('a feed that fails says why and the search stays the dialog`s', async () => {
      const s = setup();
      const search = await started(s);
      search.emit({ kind: 'found', hits: [hit('a')] });
      search.emit(done(1));

      const fed = await s.finder.feed({
        openResults: async () => {
          throw new IpcFailure({ code: 'notFound', message: 'this search is no longer kept' });
        },
      });
      s.finder.close();

      expect(fed).toBe(false);
      expect(s.calls('find_forget')).toEqual([{ id: 1 }]);
    });
  });
});
