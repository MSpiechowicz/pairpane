import { describe, expect, it, vi } from 'vitest';

import type { ByteDifference } from '$lib/ipc/bindings/ByteDifference';

import { fakeBackend } from '../../../tests/panes';
import { diffRow, groupDigits, hexOffset, sizeLine } from './hexDiff';
import { CLOSED_NOTICE, HexDiff } from './hexDiff.svelte';

const bytes = (...values: number[]) => Uint8Array.from(values);

/** The next (previous) offset that differs, as the backend defines it: the length of the shorter file counts as a difference up to the longer one's end. */
function nextDifference(left: Uint8Array, right: Uint8Array, from: number, forward: boolean): ByteDifference {
  const differs = (at: number) => at >= Math.min(left.length, right.length) || left[at] !== right[at];
  const end = Math.max(left.length, right.length);

  if (forward) {
    for (let at = Math.max(0, from); at < end; at += 1) {
      if (differs(at)) {
        return { kind: 'found', offset: at };
      }
    }
  } else {
    for (let at = Math.min(from, end) - 1; at >= 0; at -= 1) {
      if (differs(at)) {
        return { kind: 'found', offset: at };
      }
    }
  }

  return { kind: 'none' };
}

type Asked = { left: number; right: number; from: number; forward: boolean };

/** Two files on a fake backend: `l` is session 1, `r` session 2; `ask` may answer `diff_next_difference` instead of the real search. */
async function compared(left: Uint8Array, right: Uint8Array, ask?: (args: Asked) => unknown) {
  const files = [left, right];
  const backend = fakeBackend({
    view_open: ((args: { name: string }) => {
      const session = args.name === 'l' ? 1 : 2;
      return { session, name: args.name, size: files[session - 1].length, modified: null, kind: { kind: 'binary' }, encoding: '' };
    }) as never,
    view_bytes: ((args: { session: number; offset: number; length: number }) =>
      files[args.session - 1].slice(args.offset, args.offset + args.length).buffer) as never,
    diff_next_difference: ((args: Asked) => ask?.(args) ?? nextDifference(left, right, args.from, args.forward)) as never,
  });
  const diff = new HexDiff(backend.ipc, { path: '/one', name: 'l' }, { path: '/two', name: 'r' });
  await vi.waitFor(() => expect(diff.status).toBe('ready'));
  const asked = () => backend.invoke.mock.calls.filter(([name]) => name === 'diff_next_difference').map(([, args]) => args as unknown as Asked);

  return { backend, diff, asked };
}

describe('the rows of two files', () => {
  it('marks the bytes that differ in both columns, and only those', () => {
    const row = diffRow(0, { bytes: bytes(1, 2, 3), base: 0 }, { bytes: bytes(1, 9, 3), base: 0 }, 8);

    expect(row.label).toBe('00000000');
    expect(row.left.slice(0, 3).map((cell) => [cell.hex, cell.differs])).toEqual([['01', false], ['02', true], ['03', false]]);
    expect(row.right.slice(0, 3).map((cell) => [cell.hex, cell.differs])).toEqual([['01', false], ['09', true], ['03', false]]);
  });

  it('leaves the cells past the end of the shorter file blank, and marks the bytes the longer one has there', () => {
    const row = diffRow(16, { bytes: bytes(7, 7), base: 16 }, { bytes: bytes(7), base: 16 }, 8);

    expect(row.right[1]).toMatchObject({ blank: true, hex: '', differs: false });
    expect(row.right[0]).toMatchObject({ blank: false, differs: false });
    expect(row.left[1]).toMatchObject({ blank: false, hex: '07', differs: true });
    expect(row.right).toHaveLength(16);
    expect(row.right.slice(2).every((cell) => cell.blank)).toBe(true);
  });

  it('marks the difference that was gone to', () => {
    const row = diffRow(0, { bytes: bytes(1, 2), base: 0 }, { bytes: bytes(1, 3), base: 0 }, 8, 1);

    expect(row.left.map((cell) => cell.current).slice(0, 2)).toEqual([false, true]);
  });

  it('writes the sizes with groups of digits, and says where the files first differ, or that they do not', () => {
    expect(groupDigits(12345)).toBe('12 345');
    expect(groupDigits(999)).toBe('999');
    expect(groupDigits(1_234_567_890)).toBe('1 234 567 890');
    expect(hexOffset(0x1a2b)).toBe('0x1A2B');
    expect(sizeLine(12345, 12400, 0x1a2b)).toBe('12 345 bytes / 12 400 bytes, first difference at 0x1A2B');
    expect(sizeLine(1, 1, 'none')).toBe('The files are identical.');
    expect(sizeLine(1, 20, undefined)).toBe('1 byte / 20 bytes, looking for the first difference…');
  });
});

describe('the hex diff', () => {
  it('finds the first difference and says nothing is different for equal files', async () => {
    const { diff } = await compared(bytes(1, 2, 3, 4), bytes(1, 2, 9, 4));
    diff.begin();
    await vi.waitFor(() => expect(diff.first).toBe(2));

    const same = await compared(bytes(5, 5), bytes(5, 5));
    same.diff.begin();
    await vi.waitFor(() => expect(same.diff.first).toBe('none'));
  });

  it('counts the bytes of the longer file past the shorter one as a difference', async () => {
    const { diff } = await compared(bytes(1, 2, 3), bytes(1, 2, 3, 4, 5));
    diff.begin();

    await vi.waitFor(() => expect(diff.first).toBe(3));
  });

  it('shows the same rows of both files, laid out to the longer one', async () => {
    const long = Uint8Array.from({ length: 400 }, (_, index) => index % 251);
    const { diff } = await compared(long, long.slice(0, 40));
    diff.setRows(4);

    await diff.lead.moveTo(100_000);

    expect(diff.lead.top).toBe(diff.follower.top);
    expect(diff.lead.limit).toBe(diff.follower.limit);
    expect(diff.top).toBe(diff.lead.limit);
    await vi.waitFor(() => expect(diff.lines.length).toBeGreaterThan(0));
    const last = diff.lines.at(-1)!;
    expect(last.right.every((cell) => cell.blank)).toBe(true);
    expect(last.left.some((cell) => cell.differs)).toBe(true);
  });

  it('goes to the next and the previous difference, and says when there is none', async () => {
    const left = Uint8Array.from({ length: 64 }, () => 0);
    const right = left.slice();
    right[3] = 1;
    right[40] = 1;
    const { diff } = await compared(left, right);

    diff.next();
    await vi.waitFor(() => expect(diff.current).toBe(3));
    diff.next();
    await vi.waitFor(() => expect(diff.current).toBe(40));
    diff.next();
    await vi.waitFor(() => expect(diff.message).toBe('No further difference.'));
    expect(diff.current).toBe(40);

    diff.previous();
    await vi.waitFor(() => expect(diff.current).toBe(3));
    expect(diff.message).toBeUndefined();
    diff.previous();
    await vi.waitFor(() => expect(diff.message).toBe('No earlier difference.'));
    expect(diff.current).toBe(3);
  });

  it('asks again from where the backend stopped until it answers, and shows how far it has got', async () => {
    const steps: Asked['from'][] = [];
    const releases: ((answer: ByteDifference) => void)[] = [];
    const { diff, asked } = await compared(bytes(0), bytes(1), (args) => {
      steps.push(args.from);
      return new Promise<ByteDifference>((resolve) => releases.push(resolve));
    });

    diff.next();
    await vi.waitFor(() => expect(releases).toHaveLength(1));
    releases[0]({ kind: 'searching', from: 0x4000_0000 });
    await vi.waitFor(() => expect(releases).toHaveLength(2));
    expect(diff.searching).toBe(0x4000_0000);
    releases[1]({ kind: 'searching', from: 0x8000_0000 });
    await vi.waitFor(() => expect(releases).toHaveLength(3));
    releases[2]({ kind: 'found', offset: 0x8000_0010 });

    await vi.waitFor(() => expect(diff.current).toBe(0x8000_0010));
    expect(steps).toEqual([0, 0x4000_0000, 0x8000_0000]);
    expect(asked().every((args) => args.forward && args.left === 1 && args.right === 2)).toBe(true);
    expect(diff.searching).toBeUndefined();
  });

  it('drops the answer of a search that a newer one has replaced', async () => {
    const pending: { args: Asked; resolve: (answer: ByteDifference) => void }[] = [];
    const { diff } = await compared(bytes(0, 0, 0), bytes(0, 0, 0), (args) => new Promise((resolve) => pending.push({ args, resolve })));

    diff.next();
    await vi.waitFor(() => expect(pending).toHaveLength(1));
    diff.next();
    await vi.waitFor(() => expect(pending).toHaveLength(2));

    pending[0].resolve({ kind: 'found', offset: 1 });
    pending[1].resolve({ kind: 'found', offset: 2 });

    await vi.waitFor(() => expect(diff.current).toBe(2));
    expect(diff.searching).toBeUndefined();
  });

  it('stops asking when it is closed, and ends both sessions', async () => {
    const releases: ((answer: ByteDifference) => void)[] = [];
    const { diff, backend, asked } = await compared(bytes(0), bytes(1), () => new Promise((resolve) => releases.push(resolve)));

    diff.next();
    await vi.waitFor(() => expect(releases).toHaveLength(1));
    diff.close();
    releases[0]({ kind: 'searching', from: 100 });
    await new Promise((resolve) => setTimeout(resolve, 10));

    expect(asked()).toHaveLength(1);
    expect(backend.invoke.mock.calls.filter(([name]) => name === 'view_close').map(([, args]) => args)).toEqual([{ session: 1 }, { session: 2 }]);
    expect(diff.current).toBeUndefined();
  });

  it('tells that the comparison was closed when the backend no longer knows the sessions', async () => {
    const { diff } = await compared(bytes(0), bytes(1), () => Promise.reject({ code: 'notFound', message: 'there is no such viewer session' }));

    diff.next();

    await vi.waitFor(() => expect(diff.message).toBe(CLOSED_NOTICE));
    expect(diff.searching).toBeUndefined();
  });

  it('shows another failure in the backend`s words', async () => {
    const { diff } = await compared(bytes(0), bytes(1), () => Promise.reject({ code: 'io', message: 'The disk failed.' }));

    diff.previous();

    await vi.waitFor(() => expect(diff.message).toBe('The disk failed.'));
  });

  it('fails when a file cannot be opened', async () => {
    const backend = fakeBackend({ view_open: (() => Promise.reject({ code: 'notFound', message: 'That file is gone.' })) as never });
    const diff = new HexDiff(backend.ipc, { path: '/one', name: 'l' }, { path: '/two', name: 'r' });

    await vi.waitFor(() => expect(diff.status).toBe('failed'));

    expect(diff.failure).toBe('That file is gone.');
  });
});
