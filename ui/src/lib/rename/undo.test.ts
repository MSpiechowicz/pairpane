import { describe, expect, it, vi } from 'vitest';

import type { JobView } from '$lib/ipc/bindings/JobView';
import { IpcFailure } from '$lib/ipc/transport';
import type { JobResult } from '$lib/ops/transfers.svelte';

import { job, opsBackend } from '../../../tests/ops';
import { type RenameReport, RenameReports, renamed, restored } from './undo';

const rename = (id: number, items = 3): JobView => job(id, { kind: 'rename', items });
const ended = (extra: Partial<JobResult> = {}): JobResult => ({ outcome: { kind: 'done' }, skipped: 0, errors: [], moreErrors: 0, ...extra });
const error = (path: string) => ({ path, error: { code: 'io' as const, message: 'Could not rename.' } });

describe('renamed', () => {
  it('says how many were renamed and offers to undo the job', () => {
    expect(renamed(rename(4, 3), ended())).toEqual({ text: 'Renamed 3 items.', error: false, undo: 4 });
    expect(renamed(rename(4, 1), ended())).toEqual({ text: 'Renamed 1 item.', error: false, undo: 4 });
  });

  it('counts what failed out of the renamed items and tells it, still offering the undo of the rest', () => {
    expect(renamed(rename(4, 5), ended({ errors: [error('a'), error('b')], moreErrors: 1 }))).toEqual({
      text: 'Renamed 2 items. 3 items could not be renamed.',
      error: true,
      undo: 4,
    });
  });

  it('offers no undo when nothing was renamed', () => {
    expect(renamed(rename(4, 1), ended({ errors: [error('a')] }))).toMatchObject({ text: 'Renamed 0 items. 1 item could not be renamed.', undo: null });
  });

  it('a cancelled rename tells how far it got and offers the undo of that', () => {
    const cancelled = ended({ outcome: { kind: 'cancelled' } });

    expect(renamed(job(4, { kind: 'rename', items: 5, doneFiles: 2 }), cancelled)).toEqual({
      text: 'The rename was cancelled after 2 items.',
      error: false,
      undo: 4,
    });
    expect(renamed(job(4, { kind: 'rename', items: 5, doneFiles: 1 }), cancelled)).toMatchObject({ text: 'The rename was cancelled after 1 item.', undo: 4 });
  });

  it('a cancelled rename leaves out the items that failed before it, and offers no undo when none was renamed', () => {
    const cancelled = ended({ outcome: { kind: 'cancelled' }, errors: [error('a')] });

    expect(renamed(job(4, { kind: 'rename', items: 5, doneFiles: 3 }), cancelled)).toMatchObject({ text: 'The rename was cancelled after 2 items.', undo: 4 });
    expect(renamed(job(4, { kind: 'rename', items: 5, doneFiles: 1 }), cancelled)).toEqual({ text: 'The rename was cancelled.', error: false, undo: null });
    expect(renamed(rename(4, 5), ended({ outcome: { kind: 'cancelled' } }))).toEqual({ text: 'The rename was cancelled.', error: false, undo: null });
  });

  it('a failed job tells its message and offers no undo', () => {
    const outcome = { kind: 'failed' as const, error: { code: 'io' as const, message: 'Disk full.' } };

    expect(renamed(rename(4), ended({ outcome }))).toEqual({ text: 'Disk full.', error: true, undo: null });
  });
});

describe('restored', () => {
  it('tells what was put back and what was left, with no undo of an undo', () => {
    expect(restored(rename(9, 3), ended({ skipped: 1 }))).toEqual({ text: 'Restored 2, skipped 1.', error: false, undo: null });
    expect(restored(rename(9, 3), ended())).toEqual({ text: 'Restored 3 items.', error: false, undo: null });
  });

  it('a cancelled undo counts only what it got to', () => {
    const cancelled = ended({ outcome: { kind: 'cancelled' } });

    expect(restored(job(9, { kind: 'rename', items: 5, doneFiles: 2 }), cancelled)).toEqual({
      text: 'The undo was cancelled. Restored 2 items.',
      error: false,
      undo: null,
    });
  });

  it('tells what could not be restored', () => {
    expect(restored(rename(9, 3), ended({ errors: [error('a')] }))).toEqual({
      text: 'Restored 2 items. 1 item could not be restored.',
      error: true,
      undo: null,
    });
  });
});

describe('RenameReports', () => {
  function reports() {
    const told: RenameReport[] = [];
    const backend = opsBackend({ ops_rename_undo: (() => 8) as never });
    return { told, reports: new RenameReports((report) => told.push(report)), ...backend };
  }

  it('tells the end of a rename, and nothing of other jobs', () => {
    const { told, reports: all } = reports();

    all.ended(job(1, { kind: 'copy' }), ended());
    all.ended(rename(2, 2), ended());

    expect(told).toEqual([{ text: 'Renamed 2 items.', error: false, undo: 2 }]);
  });

  it('asks for the undo of the job it is given, and tells the job it started as an undo', async () => {
    const { told, reports: all, ipc, calls } = reports();
    all.ended(rename(2, 2), ended());

    await all.undo(ipc, 2);
    all.ended(rename(8, 2), ended({ skipped: 1 }));

    expect(calls('ops_rename_undo')).toEqual([{ job: 2 }]);
    expect(told.at(-1)).toEqual({ text: 'Restored 1, skipped 1.', error: false, undo: null });
  });

  it('a short undo may end before its job is known: it is told when the answer is there, as an undo', async () => {
    const told: RenameReport[] = [];
    const all = new RenameReports((report) => told.push(report));
    let answer: (id: number) => void = () => {};
    const ipc = { invoke: vi.fn(() => new Promise<number>((resolve) => (answer = resolve))) };

    const undoing = all.undo(ipc as never, 2);
    all.ended(rename(8, 2), ended());
    expect(told).toEqual([]);
    answer(8);
    await undoing;

    expect(told).toEqual([{ text: 'Restored 2 items.', error: false, undo: null }]);
  });

  it('a refused undo rejects, and what ended meanwhile is told as a rename', async () => {
    const told: RenameReport[] = [];
    const all = new RenameReports((report) => told.push(report));
    const refusal = new IpcFailure({ code: 'invalidInput', message: 'The rename is still running.' });
    let refuse: (error: unknown) => void = () => {};
    const ipc = { invoke: vi.fn(() => new Promise((_, reject) => (refuse = reject))) };

    const undoing = all.undo(ipc as never, 2);
    all.ended(rename(3, 1), ended());
    refuse(refusal);

    await expect(undoing).rejects.toBe(refusal);
    expect(told).toEqual([{ text: 'Renamed 1 item.', error: false, undo: 3 }]);
  });
});
