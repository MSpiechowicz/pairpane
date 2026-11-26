import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Transport } from '$lib/ipc/transport';

import { archiveStart, arrive, entry, fakeBackend, resultsStart } from '../../../tests/panes';
import { picture, refusal } from '../../../tests/thumbs';
import { Listing } from './listing.svelte';
import { THUMB_SIZE, type ThumbTarget, targetOf } from './thumbs';
import { Thumbs } from './thumbs.svelte';

type Ask = { path: string; name: string; size: number };

const target = (name: string, folder = '/pics'): ThumbTarget => ({ key: `${folder}/${name}`, folder, name });
const names = (...list: string[]) => list.map((name) => target(name));

/** A model over a transport whose `thumb_get` is answered by `answer`; `asked` lists what it was asked, in order. */
function world(answer: (ask: Ask) => unknown, options?: ConstructorParameters<typeof Thumbs>[1]) {
  const asked: Ask[] = [];
  const invoke = vi.fn(async (_name: string, args: Ask) => {
    asked.push(args);
    return answer(args);
  });
  const thumbs = new Thumbs({ invoke } as unknown as Transport, options);

  return { thumbs, asked, askedNames: () => asked.map((ask) => ask.name) };
}

/** Lets the answers that are ready be taken in. */
const settled = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('Thumbs', () => {
  it('asks for the folder, the name and the size, and keeps the pixels that come back', async () => {
    const { thumbs, asked } = world(() => picture(2, 1, [1, 2, 3, 4, 5, 6, 7, 8]));
    thumbs.want([target('a.jpg', '/pics/trip')]);
    await settled();

    expect(asked).toEqual([{ path: '/pics/trip', name: 'a.jpg', size: THUMB_SIZE }]);
    const shown = thumbs.get(target('a.jpg', '/pics/trip'));
    expect(shown).toMatchObject({ width: 2, height: 1 });
    expect(Array.from(shown?.pixels ?? [])).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
  });

  it('has nothing to show for a picture not asked for, or not answered yet', async () => {
    const { thumbs } = world(() => new Promise(() => {}));

    expect(thumbs.get(undefined)).toBeUndefined();
    thumbs.want(names('a.jpg'));
    await settled();

    expect(thumbs.get(target('a.jpg'))).toBeUndefined();
  });

  it('has three requests out at the most, and sends the next one as an answer comes', async () => {
    const out = new Map<string, (value: ArrayBuffer) => void>();
    const { thumbs, askedNames } = world(({ name }) => new Promise<ArrayBuffer>((resolve) => out.set(name, resolve)));

    thumbs.want(names('a', 'b', 'c', 'd', 'e'));
    await settled();
    expect(askedNames()).toEqual(['a', 'b', 'c']);

    out.get('b')?.(picture(1, 1));
    await settled();
    expect(askedNames()).toEqual(['a', 'b', 'c', 'd']);

    out.get('a')?.(picture(1, 1));
    out.get('c')?.(picture(1, 1));
    await settled();
    expect(askedNames()).toEqual(['a', 'b', 'c', 'd', 'e']);
  });

  it('asks in the order it is told, and what was queued but is no longer wanted is never asked for', async () => {
    const out = new Map<string, (value: ArrayBuffer) => void>();
    const { thumbs, askedNames } = world(({ name }) => new Promise<ArrayBuffer>((resolve) => out.set(name, resolve)));

    thumbs.want(names('a', 'b', 'c', 'd', 'e', 'f'));
    await settled();

    // The list scrolled: d and e are out of view, x and y are the ones in view now.
    thumbs.want(names('x', 'y', 'f'));
    out.get('a')?.(picture(1, 1));
    await settled();
    expect(askedNames()).toEqual(['a', 'b', 'c', 'x']);

    out.get('b')?.(picture(1, 1));
    out.get('c')?.(picture(1, 1));
    await settled();
    expect(askedNames()).toEqual(['a', 'b', 'c', 'x', 'y', 'f']);
  });

  it('keeps the picture of a request that was out when the list moved on', async () => {
    const out = new Map<string, (value: ArrayBuffer) => void>();
    const { thumbs, askedNames } = world(({ name }) => new Promise<ArrayBuffer>((resolve) => out.set(name, resolve)));

    thumbs.want(names('a'));
    await settled();
    thumbs.want(names('x'));
    out.get('a')?.(picture(3, 3));
    await settled();

    expect(thumbs.get(target('a'))).toMatchObject({ width: 3 });
    thumbs.want(names('a'));
    await settled();
    expect(askedNames()).toEqual(['a', 'x']);
  });

  it('does not ask again for what it has, and a hit needs no request', async () => {
    const { thumbs, asked } = world(() => picture(1, 1));

    thumbs.want(names('a', 'b'));
    await settled();
    expect(asked).toHaveLength(2);

    thumbs.want(names('b', 'a'));
    await settled();
    expect(asked).toHaveLength(2);
    expect(thumbs.get(target('a'))).toBeDefined();
  });

  it('asks for nothing new for a while when the backend is busy, longer each time it says so again', async () => {
    let refusals = 2;
    const { thumbs, askedNames } = world(({ name }) => {
      if (name === 'a' && refusals > 0) {
        refusals -= 1;
        throw refusal('busy');
      }

      return picture(1, 1);
    });

    thumbs.want(names('a', 'b', 'c', 'd'));
    await settled();
    // The backend is full: d waits with a, though b and c have answered.
    expect(askedNames()).toEqual(['a', 'b', 'c']);

    await vi.advanceTimersByTimeAsync(300);
    expect(askedNames()).toEqual(['a', 'b', 'c', 'a', 'd']); // busy again: the next wait is twice as long

    await vi.advanceTimersByTimeAsync(599);
    expect(askedNames()).toHaveLength(5);
    await vi.advanceTimersByTimeAsync(1);
    expect(askedNames()).toEqual(['a', 'b', 'c', 'a', 'd', 'a']);
    expect(thumbs.get(target('a'))).toBeDefined();
  });

  it('asks again later for a picture not read for a passing reason, and never for one that is not a picture', async () => {
    let failures = 0;
    const { thumbs, askedNames } = world(({ name }) => {
      if (name === 'flaky' && failures === 0) {
        failures += 1;
        throw refusal('io');
      }
      if (name === 'notes') {
        throw refusal('unsupported');
      }

      return picture(1, 1);
    });

    thumbs.want(names('flaky', 'notes'));
    await settled();
    expect(thumbs.get(target('flaky'))).toBeUndefined();

    thumbs.want(names('flaky', 'notes'));
    await settled();
    expect(askedNames()).toEqual(['flaky', 'notes']); // neither is asked for again yet

    await vi.advanceTimersByTimeAsync(10_000);
    thumbs.want(names('flaky', 'notes'));
    await settled();

    expect(askedNames()).toEqual(['flaky', 'notes', 'flaky']);
    expect(thumbs.get(target('flaky'))).toBeDefined();
  });

  it('does not ask again for a picture that went out of view while the backend was busy', async () => {
    let refused = false;
    const { thumbs, askedNames } = world(() => {
      if (!refused) {
        refused = true;
        throw refusal('busy');
      }

      return picture(1, 1);
    });

    thumbs.want(names('a'));
    await settled();
    thumbs.want(names('x'));
    await vi.advanceTimersByTimeAsync(1000);

    expect(askedNames()).toEqual(['a', 'x']);
  });

  it.each(['unsupported', 'invalidInput', 'io'])('has no picture, and does not ask again, after the backend says %s', async (code) => {
    const { thumbs, asked } = world(() => {
      throw refusal(code);
    });

    thumbs.want(names('a'));
    await settled();
    expect(thumbs.get(target('a'))).toBeUndefined();

    thumbs.want(names('a'));
    await vi.advanceTimersByTimeAsync(1000);
    expect(asked).toHaveLength(1);
  });

  it('has no picture for an answer that is not one', async () => {
    const { thumbs, asked } = world(() => new ArrayBuffer(5));

    thumbs.want(names('a'));
    await settled();

    expect(thumbs.get(target('a'))).toBeUndefined();
    thumbs.want(names('a'));
    await settled();
    expect(asked).toHaveLength(1);
  });

  it('keeps a bounded number of answers: the one wanted the longest ago goes first', async () => {
    const { thumbs, askedNames } = world(() => picture(1, 1), { max: 3 });

    thumbs.want(names('a', 'b', 'c'));
    await settled();
    thumbs.want(names('a')); // a is wanted again, so b is now the oldest
    thumbs.want(names('d'));
    await settled();
    expect(askedNames()).toEqual(['a', 'b', 'c', 'd']);

    thumbs.want(names('a', 'c', 'd'));
    await settled();
    expect(askedNames()).toHaveLength(4);

    thumbs.want(names('b'));
    await settled();
    expect(askedNames()).toEqual(['a', 'b', 'c', 'd', 'b']);
    expect(thumbs.get(target('b'))).toBeDefined();
  });

  it('counts what could not be shown among the answers it keeps', async () => {
    const { thumbs, asked } = world(({ name }) => {
      if (name === 'a') {
        throw refusal('unsupported');
      }

      return picture(1, 1);
    }, { max: 2 });

    thumbs.want(names('a', 'b', 'c'));
    await settled();
    expect(asked).toHaveLength(3);

    thumbs.want(names('a'));
    await settled();
    expect(asked).toHaveLength(4); // a was the oldest and went
  });
});

describe('targetOf', () => {
  /** A listing of `/pics` with these entries. */
  async function shown(entries = [entry('a.jpg')]) {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.open('/pics');
    arrive(backend.last(), '/pics', entries);

    return { backend, listing, entries };
  }

  it.each(['a.jpg', 'a.JPG', 'a.jpeg', 'a.png', 'a.gif', 'a.webp', 'a.bmp', 'a.heic', 'a.HEIF', 'many.dots.png'])(
    'asks for the thumbnail of the picture %s',
    async (name) => {
      const { listing } = await shown();

      expect(targetOf(listing, entry(name), THUMB_SIZE)).toMatchObject({ folder: '/pics', name });
    },
  );

  it.each(['a.txt', 'a.svg', 'a.pdf', 'jpg', 'a.jpg.bak', 'a.jpgx', '.png'])('gives %s its icon', async (name) => {
    const { listing } = await shown();

    expect(targetOf(listing, entry(name), THUMB_SIZE)).toBeUndefined();
  });

  it('gives a folder and a row that is not there its icon, whatever the name', async () => {
    const { listing } = await shown();

    expect(targetOf(listing, entry('album.jpg', 'dir'), THUMB_SIZE)).toBeUndefined();
    expect(targetOf(listing, entry('other.png', 'other'), THUMB_SIZE)).toBeUndefined();
    expect(targetOf(listing, undefined, THUMB_SIZE)).toBeUndefined();
  });

  it('tells a changed file (its size, its time), another folder and another size from the same file', async () => {
    const { listing } = await shown();
    const key = (extra: Parameters<typeof entry>[2], size = THUMB_SIZE) => targetOf(listing, entry('a.jpg', 'file', extra), size)?.key;

    const keys = new Set([key({}), key({ size: 5 }), key({ modified: 7 }), key({}, 64)]);
    expect(keys.size).toBe(4);
    expect(key({ size: 5, modified: 7 })).toBe(key({ size: 5, modified: 7 }));
  });

  it('has the folder that holds a search result: the names are paths below the folder searched', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.openResults(7);
    backend.last().emit(resultsStart('/home/me', 2, 7));

    expect(targetOf(listing, entry('docs/deep/a.png'), THUMB_SIZE)).toMatchObject({ folder: '/home/me/docs/deep', name: 'a.png' });
    expect(targetOf(listing, entry('top.png'), THUMB_SIZE)).toMatchObject({ folder: '/home/me', name: 'top.png' });
  });

  it('has none inside an archive', async () => {
    const backend = fakeBackend();
    const listing = new Listing(backend.ipc, 'left');
    await listing.openArchive({ archive: '/pics/a.zip', dir: '' });
    backend.last().emit(archiveStart('/pics', 1, { archive: '/pics/a.zip', dir: '' }));

    expect(listing.readonly).toBe(true);
    expect(targetOf(listing, entry('a.jpg'), THUMB_SIZE)).toBeUndefined();
  });

  it('has none before the folder is listed', () => {
    const listing = new Listing(fakeBackend().ipc, 'left');

    expect(targetOf(listing, entry('a.jpg'), THUMB_SIZE)).toBeUndefined();
  });
});
