import { describe, expect, it } from 'vitest';

import type { JobView } from '$lib/ipc/bindings/JobView';
import type { JobResult } from '$lib/ops/transfers.svelte';
import type { UndoReport } from '$lib/ops/undoReports';

import { job, opsBackend } from '../../../tests/ops';
import { SyncReports, synced } from './undo';

const sync = (id: number, items = 3): JobView => job(id, { kind: 'sync', items });
const ended = (extra: Partial<JobResult> = {}): JobResult => ({ outcome: { kind: 'done' }, skipped: 0, errors: [], moreErrors: 0, ...extra });

describe('synced', () => {
  it('says how many items were synchronized, which changed and were left, and offers the undo', () => {
    expect(synced(sync(4, 3), ended())).toEqual({ text: 'Synchronized 3 items.', error: false, undo: 4 });
    expect(synced(sync(4, 3), ended({ skipped: 1 }))).toEqual({
      text: 'Synchronized 2 items. 1 item had changed and was left alone.',
      error: false,
      undo: 4,
    });
  });

  it('offers no undo when nothing was done, or the job failed', () => {
    expect(synced(sync(4, 1), ended({ skipped: 1 })).undo).toBeNull();
    expect(synced(sync(4, 1), { ...ended(), outcome: { kind: 'failed', error: { code: 'io', message: 'The disk is full.' } } })).toEqual({
      text: 'The disk is full.',
      error: true,
      undo: null,
    });
  });
});

describe('SyncReports', () => {
  it('asks for the undo of the sync and tells the undo job as a restore when it ends', async () => {
    const told: UndoReport[] = [];
    const reports = new SyncReports((report) => told.push(report));
    const backend = opsBackend({ ops_sync_undo: () => 9 });

    reports.ended(sync(4, 2), ended());
    await reports.undo(backend.ipc, 4);
    reports.ended(job(9, { kind: 'sync', items: 2 }), ended({ skipped: 1 }));
    reports.ended(job(5, { kind: 'rename', items: 2 }), ended());

    expect(backend.calls('ops_sync_undo')).toEqual([{ job: 4 }]);
    expect(told.map((report) => report.text)).toEqual(['Synchronized 2 items.', 'Undid 1, skipped 1.']);
  });

  it('offers no undo for a sync that wrote to no folder, and offers it again after a cancelled undo', async () => {
    const told: UndoReport[] = [];
    const reports = new SyncReports((report) => told.push(report));
    const backend = opsBackend({ ops_sync_undo: () => 9 });

    reports.expect(3, false);
    reports.ended(sync(3, 2), ended());
    reports.ended(sync(4, 2), ended());
    await reports.undo(backend.ipc, 4);
    reports.ended(job(9, { kind: 'sync', items: 2, doneFiles: 1 }), ended({ outcome: { kind: 'cancelled' } }));

    expect(told.map((report) => report.undo)).toEqual([null, 4, 4]);
    expect(told[2].text).toBe('The undo was cancelled. Undid 1 item.');
  });
});
