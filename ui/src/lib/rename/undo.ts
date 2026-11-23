import type { JobView } from '$lib/ipc/bindings/JobView';
import { failedOf, reportOf, restoredOf, type UndoReport, UndoReports } from '$lib/ops/undoReports';
import type { JobResult } from '$lib/ops/transfers.svelte';

/** What to tell the user about a rename job that ended; `undo` is the job to take back, when that can be offered. */
export type RenameReport = UndoReport;

const items = (n: number) => (n === 1 ? '1 item' : `${n} items`);

/** A rename that ended: what was renamed, and the job to undo it with (not when nothing was, also after a cancel). */
export function renamed(job: JobView, result: JobResult): RenameReport {
  return reportOf(job, result, (done, cancelled) => {
    if (cancelled) {
      return { text: done > 0 ? `The rename was cancelled after ${items(done)}.` : 'The rename was cancelled.', error: false };
    }

    const failures = failedOf(result, 'renamed');
    return { text: `Renamed ${items(done)}.${failures}`, error: failures !== '' };
  });
}

/** An undo of a rename that ended: what was put back, and what was left because the names had changed since. */
export const restored = restoredOf;

/** Tells how renames and their undos went (see `UndoReports`). */
export class RenameReports extends UndoReports {
  constructor(report: (report: RenameReport) => void) {
    super({ kind: 'rename', command: 'ops_rename_undo', done: renamed, undone: restored }, report);
  }
}
