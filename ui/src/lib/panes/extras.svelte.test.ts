import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Dimensions } from '$lib/ipc/bindings/Dimensions';
import type { Transport } from '$lib/ipc/transport';

import { refusal } from '../../../tests/thumbs';
import type { ExtraTarget } from './extras';
import { Extras, MAX_NAMES } from './extras.svelte';

const target = (name: string, folder = '/pics', picture = true): ExtraTarget => ({ key: `${folder}/${name}`, folder, name, picture });
const pictures = (...list: string[]) => ({ dimensions: list.map((name) => target(name)), checksums: [], git: [], reloads: 0 });
const files = (...list: string[]) => ({ dimensions: [], checksums: list.map((name) => target(name)), git: [], reloads: 0 });

type Ask = { path: string; name?: string; names?: string[] };

/** An `Extras` over a transport answered by `answer` (per command); `asked` lists what it was asked, in order. */
function world(answer: (command: string, ask: Ask) => unknown, options?: ConstructorParameters<typeof Extras>[1]) {
  const asked: { command: string; ask: Ask }[] = [];
  const invoke = vi.fn(async (command: string, ask: Ask) => {
    asked.push({ command, ask });
    return answer(command, ask);
  });

  return {
    extras: new Extras({ invoke } as unknown as Transport, options),
    asked,
    batches: () => asked.filter(({ command }) => command === 'columns_dimensions').map(({ ask }) => ask),
    sums: () => asked.filter(({ command }) => command === 'columns_checksum').map(({ ask }) => ask.name),
  };
}

/** A request the test settles when it likes. */
const held = () => {
  const out = new Map<string, { resolve: (value: unknown) => void; reject: (reason: unknown) => void }>();
  const answer = (name: string) => new Promise((resolve, reject) => out.set(name, { resolve, reject }));

  return { out, answer };
};

/** Lets the answers that are ready be taken in. */
const settled = () => vi.advanceTimersByTimeAsync(0);

const dims = (width: number, height: number): Dimensions => ({ width, height });

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('Extras: dimensions', () => {
  it('asks for the pictures of a folder in one request, and the answers are the ones of the names, in order', async () => {
    const { extras, batches } = world(() => [dims(640, 480), null, dims(4032, 3024)]);

    extras.want(pictures('a.jpg', 'b.png', 'c.heic'));
    await settled();

    expect(batches()).toEqual([{ path: '/pics', names: ['a.jpg', 'b.png', 'c.heic'] }]);
    expect(extras.cells(target('a.jpg')).dimensions).toEqual({ text: '640×480', tip: '640 × 480 pixels' });
    expect(extras.cells(target('b.png')).dimensions).toEqual({ text: '–', tip: 'No dimensions' });
    expect(extras.cells(target('c.heic')).dimensions.text).toBe('4032×3024');
  });

  it('shows nothing for a picture that has not been answered yet, and for what is not a picture', async () => {
    const { extras } = world(() => new Promise(() => {}));

    extras.want(pictures('a.jpg'));
    await settled();

    expect(extras.cells(target('a.jpg')).dimensions).toEqual({ text: '' });
    expect(extras.cells(target('notes.txt', '/pics', false)).dimensions).toEqual({ text: '' });
    expect(extras.cells(undefined).dimensions).toEqual({ text: '' });
  });

  it(`asks for ${MAX_NAMES} names at the most in a request, and sends the rest when it is answered`, async () => {
    const names = Array.from({ length: MAX_NAMES + 88 }, (_, i) => `p${i}.jpg`);
    const { extras, batches } = world((_, { names: asked = [] }) => asked.map(() => dims(1, 1)));

    extras.want(pictures(...names));
    await settled();

    expect(batches().map((batch) => batch.names?.length)).toEqual([MAX_NAMES, 88]);
    expect(batches()[1].names?.[0]).toBe(`p${MAX_NAMES}.jpg`);
  });

  it('makes one request for each folder (the items of a search result are in several), two at a time', async () => {
    const { out, answer } = held();
    const { extras, batches } = world((_, { path }) => answer(path));
    const mixed = { dimensions: [target('a.jpg', '/a'), target('b.jpg', '/b'), target('c.jpg', '/a'), target('d.jpg', '/d')], checksums: [], git: [], reloads: 0 };

    extras.want(mixed);
    await settled();
    expect(batches()).toEqual([
      { path: '/a', names: ['a.jpg', 'c.jpg'] },
      { path: '/b', names: ['b.jpg'] },
    ]);

    out.get('/a')?.resolve([dims(1, 2), dims(3, 4)]);
    await settled();

    expect(batches().at(-1)).toEqual({ path: '/d', names: ['d.jpg'] });
    expect(batches()).toHaveLength(3);
    expect(extras.cells(target('c.jpg', '/a')).dimensions.text).toBe('3×4');
  });

  it('leaves a name the answer is short of without dimensions, and never asks for it again', async () => {
    const { extras, batches } = world(() => [dims(2, 2)]);

    extras.want(pictures('a.jpg', 'b.jpg'));
    await settled();
    extras.want(pictures('a.jpg', 'b.jpg'));
    await settled();

    expect(batches()).toHaveLength(1);
    expect(extras.cells(target('b.jpg')).dimensions.text).toBe('–');
  });

  it('asks again for nothing it has: a hit needs no request, and a change of the file (its key) does', async () => {
    const { extras, batches } = world(() => [dims(1, 1)]);

    extras.want(pictures('a.jpg'));
    await settled();
    extras.want(pictures('a.jpg'));
    await settled();
    expect(batches()).toHaveLength(1);

    extras.want({ dimensions: [{ ...target('a.jpg'), key: 'changed' }], checksums: [], git: [], reloads: 0 });
    await settled();
    expect(batches()).toHaveLength(2);
  });

  it('keeps about as many values as it is told to, the least recently wanted going first', async () => {
    const { extras, batches } = world((_, { names = [] }) => names.map(() => dims(1, 1)), { max: 3 });

    extras.want(pictures('a.jpg', 'b.jpg'));
    await settled();
    extras.want(pictures('c.jpg', 'd.jpg'));
    await settled();
    expect(extras.cells(target('a.jpg')).dimensions.text).toBe(''); // went
    expect(extras.cells(target('b.jpg')).dimensions.text).toBe('1×1');

    extras.want(pictures('a.jpg'));
    await settled();
    expect(batches()).toHaveLength(3);
  });

  it('drops what was queued and is not wanted any more, and keeps the answer of a request that was out', async () => {
    const { out, answer } = held();
    const { extras, batches } = world((_, { path }) => answer(path));

    extras.want({ dimensions: [target('a.jpg', '/a'), target('b.jpg', '/b')], checksums: [], git: [], reloads: 0 });
    await settled();
    extras.want({ dimensions: [target('x.jpg', '/x'), target('y.jpg', '/y')], checksums: [], git: [], reloads: 0 });
    extras.want({ dimensions: [target('y.jpg', '/y')], checksums: [], git: [], reloads: 0 }); // x is gone before its turn
    out.get('/a')?.resolve([dims(7, 7)]);
    await settled();

    expect(batches()).toEqual([
      { path: '/a', names: ['a.jpg'] },
      { path: '/b', names: ['b.jpg'] },
      { path: '/y', names: ['y.jpg'] },
    ]);
    expect(extras.cells(target('a.jpg', '/a')).dimensions.text).toBe('7×7');
  });

  it('waits when the backend is busy, longer each time, and then asks for the same names again', async () => {
    let refusals = 2;
    const { extras, batches } = world(() => {
      if (refusals > 0) {
        refusals -= 1;
        throw refusal('busy');
      }

      return [dims(5, 5)];
    });

    extras.want(pictures('a.jpg'));
    await settled();
    expect(batches()).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(299);
    expect(batches()).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(batches()).toHaveLength(2); // busy again: the wait is twice as long

    await vi.advanceTimersByTimeAsync(599);
    expect(batches()).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(1);

    expect(batches()).toHaveLength(3);
    expect(extras.cells(target('a.jpg')).dimensions.text).toBe('5×5');
  });

  it('keeps nothing when the whole call fails: the names wait about 10 seconds and are asked for again if still wanted', async () => {
    let failures = 1;
    const { extras, batches } = world((_, { names = [] }) => {
      if (failures > 0) {
        failures -= 1;
        throw refusal('io');
      }

      return names.map(() => dims(9, 9));
    });

    extras.want(pictures('a.jpg', 'b.jpg'));
    await settled();
    expect(extras.cells(target('a.jpg')).dimensions.text).toBe(''); // not "none"

    extras.want(pictures('a.jpg', 'b.jpg'));
    await vi.advanceTimersByTimeAsync(9_999);
    expect(batches()).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(1);
    expect(batches()).toEqual([
      { path: '/pics', names: ['a.jpg', 'b.jpg'] },
      { path: '/pics', names: ['a.jpg', 'b.jpg'] },
    ]);
    expect(extras.cells(target('b.jpg')).dimensions.text).toBe('9×9');
  });

  it('does not ask again, after the wait, for names that are not wanted any more', async () => {
    const { extras, batches } = world(() => {
      throw refusal('io');
    });

    extras.want(pictures('a.jpg'));
    await settled();
    extras.want(pictures('x.jpg'));
    await vi.advanceTimersByTimeAsync(20_000);

    const later = batches().slice(1).flatMap((batch) => batch.names);
    expect(later).toContain('x.jpg');
    expect(later).not.toContain('a.jpg');
  });

  it('stops asking after 8 busy answers in a row, until something else is wanted', async () => {
    const { extras, batches } = world(() => {
      throw refusal('busy');
    });

    extras.want(pictures('a.jpg'));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(batches()).toHaveLength(8);

    extras.want(pictures('a.jpg')); // the same again (a scroll within the same rows)
    await vi.advanceTimersByTimeAsync(60_000);
    expect(batches()).toHaveLength(8);

    extras.want(pictures('a.jpg', 'b.jpg'));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(batches().length).toBeGreaterThan(8);
    expect(batches().at(-1)?.names).toEqual(['a.jpg', 'b.jpg']);
  });

  it('sorts by the number of pixels, and has nothing to sort by while it does not know', async () => {
    const { extras } = world(() => [dims(640, 480), null]);

    expect(extras.sortKey('dimensions', target('a.jpg'))).toBeUndefined();
    extras.want(pictures('a.jpg', 'b.jpg'));
    await settled();

    expect(extras.sortKey('dimensions', target('a.jpg'))).toBe(640 * 480);
    expect(extras.sortKey('dimensions', target('b.jpg'))).toBeUndefined();
    expect(extras.sortKey('dimensions', undefined)).toBeUndefined();
  });
});

describe('Extras: checksums', () => {
  const HEX = 'abcdef0123456789'.repeat(4);

  it('asks for one file in a request, and shows the first 12 digits with the whole checksum as the tooltip', async () => {
    const { extras, sums, asked } = world(() => HEX);

    extras.want(files('a.bin', 'b.bin'));
    await settled();

    expect(sums()).toEqual(['a.bin', 'b.bin']);
    expect(asked[0]).toEqual({ command: 'columns_checksum', ask: { path: '/pics', name: 'a.bin' } });
    expect(extras.cells(target('a.bin')).checksum).toEqual({ text: HEX.slice(0, 12), tip: HEX });
    expect(extras.sortKey('checksum', target('a.bin'))).toBe(HEX);
  });

  it('has two requests out at the most, and sends the next one as an answer comes, in the order told', async () => {
    const { out, answer } = held();
    const { extras, sums } = world((_, { name = '' }) => answer(name));

    extras.want(files('a', 'b', 'c', 'd', 'e'));
    await settled();
    expect(sums()).toEqual(['a', 'b']);

    out.get('b')?.resolve(HEX);
    await settled();
    expect(sums()).toEqual(['a', 'b', 'c']);

    out.get('a')?.resolve(HEX);
    out.get('c')?.resolve(HEX);
    await settled();
    expect(sums()).toEqual(['a', 'b', 'c', 'd', 'e']);
  });

  it('shows a dash with the reason for a file that is too big, and does not ask again', async () => {
    const { extras, sums } = world(() => {
      throw { code: 'invalidInput', message: 'the file is too big to checksum here' };
    });

    extras.want(files('big.iso'));
    await settled();
    extras.want(files('big.iso'));
    await settled();

    expect(sums()).toEqual(['big.iso']);
    expect(extras.cells(target('big.iso')).checksum).toEqual({ text: '–', tip: 'the file is too big to checksum here' });
    expect(extras.sortKey('checksum', target('big.iso'))).toBeUndefined();
  });

  it('keeps a file that is not on this computer as a reason too', async () => {
    const { extras, sums } = world(() => {
      throw { code: 'invalidInput', message: 'the file is not on this computer' };
    });

    extras.want(files('cloud.doc'));
    await settled();
    extras.want(files('cloud.doc'));
    await settled();

    expect(sums()).toEqual(['cloud.doc']);
    expect(extras.cells(target('cloud.doc')).checksum).toEqual({ text: '–', tip: 'the file is not on this computer' });
  });

  it('keeps nothing when the read fails for another reason: the file waits about 10 seconds and is asked for again', async () => {
    let failures = 1;
    const { extras, sums } = world(() => {
      if (failures > 0) {
        failures -= 1;
        throw refusal('io');
      }

      return HEX;
    });

    extras.want(files('a', 'b'));
    await settled();
    expect(sums()).toEqual(['a', 'b']);
    expect(extras.cells(target('a')).checksum).toEqual({ text: '' }); // no dash: it may still come

    await vi.advanceTimersByTimeAsync(9_999);
    extras.want(files('a', 'b'));
    expect(sums()).toEqual(['a', 'b']);

    await vi.advanceTimersByTimeAsync(1);
    expect(sums()).toEqual(['a', 'b', 'a']);
    expect(extras.cells(target('a')).checksum.text).toBe(HEX.slice(0, 12));
  });

  it('looks at the queue as soon as the earliest wait is over, not when a later one is', async () => {
    let busy = 1;
    const { extras, sums, batches } = world((command) => {
      if (command === 'columns_dimensions') {
        throw refusal('io'); // its names wait 10 seconds
      }
      if (busy > 0) {
        busy -= 1;
        throw refusal('busy'); // the checksum waits 300 ms
      }

      return HEX;
    });

    extras.want({ dimensions: [target('a.jpg')], checksums: [target('b.bin')], git: [], reloads: 0 });
    await settled();
    await vi.advanceTimersByTimeAsync(300);

    expect(sums()).toEqual(['b.bin', 'b.bin']);
    expect(batches()).toHaveLength(1);
  });

  it('stops asking after 8 busy answers in a row, until something else is wanted', async () => {
    const { extras, sums } = world(() => {
      throw refusal('busy');
    });

    extras.want(files('a'));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(sums()).toHaveLength(8);

    extras.want(files('a'));
    await vi.advanceTimersByTimeAsync(60_000);
    expect(sums()).toHaveLength(8);

    extras.want(files('b', 'a')); // scrolled to other rows
    await vi.advanceTimersByTimeAsync(60_000);
    expect(sums().length).toBeGreaterThan(8);
    expect(sums()).toContain('b');
  });

  it('writes out the characters of a reason that could pass for others', async () => {
    const { extras } = world(() => {
      throw { code: 'invalidInput', message: 'cannot read \u202Egpj.exe' };
    });

    extras.want(files('a'));
    await settled();

    expect(extras.cells(target('a')).checksum.tip).toBe('cannot read ⟨U+202E⟩gpj.exe');
  });

  it('waits when the backend is busy, longer each time, and keeps the file queued', async () => {
    let refusals = 2;
    const { extras, sums } = world((_, { name }) => {
      if (name === 'a' && refusals > 0) {
        refusals -= 1;
        throw refusal('busy');
      }

      return HEX;
    });

    extras.want(files('a', 'b', 'c'));
    await settled();
    expect(sums()).toEqual(['a', 'b']); // a is refused, b answers: c waits with a

    await vi.advanceTimersByTimeAsync(300);
    expect(sums()).toEqual(['a', 'b', 'a', 'c']); // busy again: the next wait is twice as long

    await vi.advanceTimersByTimeAsync(599);
    expect(sums()).toHaveLength(4);
    await vi.advanceTimersByTimeAsync(1);

    expect(sums()).toEqual(['a', 'b', 'a', 'c', 'a']);
    expect(extras.cells(target('a')).checksum.text).toBe(HEX.slice(0, 12));
  });

  it('keeps about as many values as it is told to, and asks again for one that went', async () => {
    const { extras, sums } = world(() => HEX, { max: 2 });

    extras.want(files('a', 'b', 'c'));
    await settled();
    extras.want(files('a'));
    await settled();

    expect(sums()).toEqual(['a', 'b', 'c', 'a']);
  });
});
