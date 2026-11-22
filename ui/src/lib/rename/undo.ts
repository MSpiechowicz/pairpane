import type { JobId } from '$lib/ipc/bindings/JobId';
import type { JobView } from '$lib/ipc/bindings/JobView';
import type { Transport } from '$lib/ipc/transport';
import type { JobResult } from '$lib/ops/transfers.svelte';

/** What to tell the user about a rename job that ended; `undo` is the job to take back, when that can be offered. */
export interface RenameReport {
  text: string;
  error: boolean;
  undo: JobId | null;
}

const items = (n: number) => (n === 1 ? '1 item' : `${n} items`);

/**
 * How many of the job's items were done: the ones that were gone over, less those that failed or were skipped. A job
 * that ended got to all its items; a cancelled one got as far as its last progress report (`doneFiles`, which counts
 * an item however it came out). That report can lag a little behind a cancel, so the count is at least this many.
 */
function doneOf(job: JobView, result: JobResult): number {
  const reached = result.outcome.kind === 'cancelled' ? job.doneFiles : job.items;
  return Math.max(0, reached - result.errors.length - result.moreErrors - result.skipped);
}

/** The failures of a job, as a sentence's end: ` 2 items could not be renamed.` */
function failed(result: JobResult, what: string): string {
  const count = result.errors.length + result.moreErrors;
  return count === 0 ? '' : ` ${items(count)} could not be ${what}.`;
}

/** A rename that ended: what was renamed, and the job to undo it with (not when nothing was, also after a cancel). */
export function renamed(job: JobView, result: JobResult): RenameReport {
  const { outcome } = result;
  if (outcome.kind === 'failed') {
    return { text: outcome.error.message, error: true, undo: null };
  }

  const done = doneOf(job, result);
  const undo = done > 0 ? job.id : null;
  if (outcome.kind === 'cancelled') {
    const text = done > 0 ? `The rename was cancelled after ${items(done)}.` : 'The rename was cancelled.';
    return { text, error: false, undo };
  }

  const failures = failed(result, 'renamed');
  return { text: `Renamed ${items(done)}.${failures}`, error: failures !== '', undo };
}

/** An undo that ended: what was put back, and what was left because the names had changed since. */
export function restored(job: JobView, result: JobResult): RenameReport {
  const { outcome } = result;
  if (outcome.kind === 'failed') {
    return { text: outcome.error.message, error: true, undo: null };
  }

  const done = doneOf(job, result);
  const failures = failed(result, 'restored');
  const skipped = result.skipped > 0 ? `Restored ${done}, skipped ${result.skipped}.` : `Restored ${items(done)}.`;
  const text = outcome.kind === 'cancelled' ? `The undo was cancelled. ${skipped}` : skipped;
  return { text: `${text}${failures}`, error: failures !== '', undo: null };
}

/**
 * Tells how renames and their undos went. A job that ends is a rename or the undo of one; which, is known only
 * from the answer to the undo that started it, and a short job may end before that answer arrives: the jobs that
 * end meanwhile wait for it.
 */
export class RenameReports {
  readonly #report: (report: RenameReport) => void;
  readonly #undone = new Set<JobId>();
  #asking = false;
  #waiting: { job: JobView; result: JobResult }[] = [];

  constructor(report: (report: RenameReport) => void) {
    this.#report = report;
  }

  /** A job ended; only renames are told. */
  ended(job: JobView, result: JobResult) {
    if (job.kind !== 'rename') {
      return;
    }

    if (this.#asking) {
      this.#waiting.push({ job, result });
    } else {
      this.#tell(job, result);
    }
  }

  /** Queues the undo of the rename `job`; a refusal rejects and the rename can be undone again. */
  async undo(ipc: Transport, job: JobId): Promise<void> {
    this.#asking = true;
    try {
      this.#undone.add(await ipc.invoke('ops_rename_undo', { job }));
    } finally {
      this.#asking = false;
      for (const waiting of this.#waiting.splice(0)) {
        this.#tell(waiting.job, waiting.result);
      }
    }
  }

  #tell(job: JobView, result: JobResult) {
    this.#report(this.#undone.has(job.id) ? restored(job, result) : renamed(job, result));
  }
}
