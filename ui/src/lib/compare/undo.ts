import type { JobId } from '$lib/ipc/bindings/JobId';
import type { JobView } from '$lib/ipc/bindings/JobView';
import { failedOf, reportOf, restoredOf, type UndoReport, UndoReports } from '$lib/ops/undoReports';
import type { JobResult } from '$lib/ops/transfers.svelte';

const items = (n: number) => (n === 1 ? '1 item' : `${n} items`);

/** A synchronization that ended: how many items it did, and the job to undo it with (not when it did none). */
export function synced(job: JobView, result: JobResult): UndoReport {
  return reportOf(job, result, (done, cancelled) => {
    const left = result.skipped;
    const skipped = left > 0 ? ` ${items(left)} had changed and ${left === 1 ? 'was' : 'were'} left alone.` : '';
    const failures = failedOf(result, 'synchronized');
    const text = cancelled ? `The synchronization was cancelled after ${items(done)}.` : `Synchronized ${items(done)}.`;
    return { text: `${text}${skipped}${failures}`, error: failures !== '' };
  });
}

/** An undo of a synchronization that ended: what it took back, and what was left because it had changed since. */
export const unsynced = (job: JobView, result: JobResult): UndoReport => restoredOf(job, result, 'Undid');

/**
 * Tells how synchronizations and their undos went (see `UndoReports`). Undo is offered only for a sync that wrote to a
 * folder (`expect` says so when the job is queued): what goes into an archive or to the Trash is not taken back by it. An
 * undo that was cancelled or failed is offered again.
 */
export class SyncReports extends UndoReports {
  readonly #folderless = new Set<JobId>();

  constructor(report: (report: UndoReport) => void) {
    super({ kind: 'sync', command: 'ops_sync_undo', done: synced, undone: unsynced, again: true, offers: (job) => !this.#folderless.has(job) }, report);
  }

  /** The sync `job` was queued; `undoable` says whether it writes to a folder at all. */
  expect(job: JobId, undoable: boolean) {
    if (!undoable) {
      this.#folderless.add(job);
    }
  }
}
