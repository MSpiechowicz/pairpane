import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SearchHit } from '$lib/ipc/bindings/SearchHit';
import type { SearchQuery } from '$lib/ipc/bindings/SearchQuery';

import { FakeFile, openedSession } from '../../../tests/view';
import { SEARCH_GAP_MS, ViewSearch } from './search.svelte';

const TEXT = 'alpha beta alpha beta ALPHA';

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

/** A search on the fake file, whose answers the test can hold back (`held`) instead of getting them at once. */
async function searching(hold = false) {
  const file = new FakeFile(TEXT);
  const held: { query: SearchQuery; progress: (n: number) => void; answer: (hit: SearchHit | null) => void }[] = [];
  const world = await openedSession(
    file,
    undefined,
    hold
      ? {
          view_search: (args: { query: SearchQuery; onProgress: (n: number) => void }) =>
            new Promise((answer) => held.push({ query: args.query, progress: args.onProgress, answer })),
        }
      : {},
  );
  const search = new ViewSearch(world.session);

  /** Runs a search and lets the gap pass, so that the next one is not joined to it. */
  const run = async (backward = false) => {
    search.run(backward);
    await vi.advanceTimersByTimeAsync(SEARCH_GAP_MS);
  };

  return { ...world, file, search, held, run };
}

describe('ViewSearch', () => {
  it('finds the next match from the top of the view, then from the end of each match, and the previous from its start', async () => {
    const { search, sent, run } = await searching();
    search.text = 'alpha';

    await run();
    expect(search.hit).toEqual({ offset: 0, len: 5 });
    await run();
    expect(search.hit).toEqual({ offset: 11, len: 5 });
    await run();
    expect(search.hit).toEqual({ offset: 22, len: 5 }); // the capital ALPHA: case does not matter by default
    await run(true);
    expect(search.hit).toEqual({ offset: 11, len: 5 });

    expect(sent('view_search').map((call) => (call as unknown as { query: SearchQuery }).query)).toMatchObject([
      { from: 0, backward: false },
      { from: 5, backward: false },
      { from: 16, backward: false },
      { from: 22, backward: true },
    ]);
  });

  it('starts from the top of the view the user has scrolled to', async () => {
    const { search, session, sent, run } = await searching();
    session.position = 12;
    search.text = 'beta';

    await run();

    expect(search.hit).toEqual({ offset: 17, len: 4 });
    expect(sent('view_search')[0]).toMatchObject({ query: { from: 12 } });
  });

  it('asks for the case to match only when told to, and says which encoding the text is in', async () => {
    const { search, session, sent, run } = await searching();
    session.encoding = 'windows-1250';
    search.text = 'ALPHA';
    search.caseSensitive = true;

    await run();

    expect(search.hit).toEqual({ offset: 22, len: 5 });
    expect(sent('view_search')[0]).toMatchObject({
      query: { needle: { kind: 'text', text: 'ALPHA', caseSensitive: true }, encoding: 'windows-1250' },
    });
  });

  it('searches for bytes when the box is hex, and does not search when it is not', async () => {
    const { search, sent, run } = await searching();
    search.hex = true;
    search.text = '62 65 74 61';

    await run();
    expect(sent('view_search')[0]).toMatchObject({ query: { needle: { kind: 'hex', bytes: [0x62, 0x65, 0x74, 0x61] } } });
    expect(search.hit).toEqual({ offset: 6, len: 4 });

    search.text = '62 6';
    expect(search.invalid).toBe(true);
    await run();
    search.text = '';
    await run();
    expect(sent('view_search')).toHaveLength(1);
  });

  it('says when nothing is found', async () => {
    const { search, run } = await searching();
    search.text = 'alpha';
    await run();
    search.text = 'zzz';
    search.edited();

    await run();

    expect(search.status).toBe('none');
    expect(search.hit).toBeUndefined();
  });

  it('a change of the needle starts the next search from the view again', async () => {
    const { search, sent, run } = await searching();
    search.text = 'beta';
    await run();
    search.text = 'alpha';
    search.edited();

    await run();

    expect(sent('view_search')[1]).toMatchObject({ query: { from: 0 } });
  });

  it('a change of the needle stops the search for the old one and drops its answer', async () => {
    const { search, held, sent } = await searching(true);
    search.text = 'alpha';
    search.run(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(search.searching).toBe(true);

    search.text = 'beta';
    search.edited();
    held[0].answer({ offset: 0, len: 5 });
    await vi.advanceTimersByTimeAsync(0);

    expect(sent('view_search_cancel')).toEqual([{ session: 1 }]);
    expect(search.status).toBe('idle');
    expect(search.hit).toBeUndefined();
  });

  it('searches asked for in quick succession are one: the first at once, the last when the gap has passed', async () => {
    const { search, sent } = await searching();
    search.text = 'alpha';

    search.run(false);
    search.run(false);
    search.run(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(sent('view_search')).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(SEARCH_GAP_MS);
    expect(sent('view_search')).toHaveLength(2);
    expect(sent('view_search')[1]).toMatchObject({ query: { backward: true } });
  });

  it('shows the progress of a long search', async () => {
    const { search, held } = await searching(true);
    search.text = 'alpha';

    search.run(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(search.searching).toBe(true);
    held[0].progress(64 * 1024 * 1024);

    expect(search.progress).toBe(64 * 1024 * 1024);
  });

  it('cancelling stops the search on the backend and drops its answer', async () => {
    const { search, held, sent } = await searching(true);
    search.text = 'alpha';
    search.run(false);
    await vi.advanceTimersByTimeAsync(0);

    search.cancel();
    held[0].answer({ offset: 0, len: 5 });
    await vi.advanceTimersByTimeAsync(0);

    expect(sent('view_search_cancel')).toEqual([{ session: 1 }]);
    expect(search.status).toBe('idle');
    expect(search.hit).toBeUndefined();
  });

  it('the answer of a search that a newer one superseded is dropped', async () => {
    const { search, held } = await searching(true);
    search.text = 'alpha';
    search.run(false);
    await vi.advanceTimersByTimeAsync(SEARCH_GAP_MS);
    search.run(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(held).toHaveLength(2);

    held[1].answer({ offset: 11, len: 5 });
    await vi.advanceTimersByTimeAsync(0);
    held[0].answer(null); // the backend ends a search it stopped with "not found"
    held[0].progress(5);
    await vi.advanceTimersByTimeAsync(0);

    expect(search.hit).toEqual({ offset: 11, len: 5 });
    expect(search.status).toBe('idle');
    expect(search.progress).toBe(0);
  });

  it('tells why a search failed, in the backend`s words', async () => {
    const world = await openedSession(new FakeFile(TEXT), undefined, {
      view_search: () => Promise.reject({ code: 'busy', message: 'the viewer is busy' }),
    });
    const search = new ViewSearch(world.session);
    search.text = 'alpha';

    search.run(false);
    await vi.advanceTimersByTimeAsync(0);

    expect(search.status).toBe('failed');
    expect(search.failure).toBe('the viewer is busy');
  });
});
