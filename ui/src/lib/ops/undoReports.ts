import type { CommandName } from '$lib/ipc/commands';
import type { JobId } from '$lib/ipc/bindings/JobId';
import type { JobView } from '$lib/ipc/bindings/JobView';
import type { Transport } from '$lib/ipc/transport';
import type { JobResult } from '$lib/ops/transfers.svelte';

/** What to tell the user about a job that ended; `undo` is the job to take back, when that can be offered. */
export interface UndoReport {
  text: string;
  error: boolean;
  undo: JobId | null;
}

/** A kind of job that can be taken back: its kind, the command that undoes it, and what to tell of it and of its undo. */
export interface Undoable {
  kind: JobView['kind'];
  command: Extract<CommandName, 'ops_rename_undo' | 'ops_sync_undo'>;
  done: (job: JobView, result: JobResult) => UndoReport;
  undone: (job: JobView, result: JobResult) => UndoReport;
  /** An undo that was cancelled or failed can be asked for again (the backend keeps what it needs). */
  again?: boolean;
  /** Whether the job can be undone at all; every job, unless it says. */
  offers?: (job: JobId) => boolean;
}

const items = (n: number) => (n === 1 ? '1 item' : `${n} items`);

/**
 * How many of the job's items were done: the ones that were gone over, less those that failed or were skipped. A job
 * that ended got to all its items; a cancelled one got as far as its last progress report (`doneFiles`, which counts
 * an item however it came out). That report can lag a little behind a cancel, so the count is at least this many.
 */
export function doneOf(job: JobView, result: JobResult): number {
  const reached = result.outcome.kind === 'cancelled' ? job.doneFiles : job.items;
  return Math.max(0, reached - result.errors.length - result.moreErrors - result.skipped);
}

/** The failures of a job, as a sentence's end: ` 2 items could not be renamed.` */
export function failedOf(result: JobResult, what: string): string {
  const count = result.errors.length + result.moreErrors;
  return count === 0 ? '' : ` ${items(count)} could not be ${what}.`;
}

/**
 * A job that can be undone, ended: the failure of one that failed, else how many items it did and the job to undo it with
 * (none when it did none), for `tell` to put in words.
 */
export function reportOf(
  job: JobView,
  result: JobResult,
  tell: (done: number, cancelled: boolean) => { text: string; error: boolean },
): UndoReport {
  const { outcome } = result;
  if (outcome.kind === 'failed') {
    return { text: outcome.error.message, error: true, undo: null };
  }

  const done = doneOf(job, result);
  return { ...tell(done, outcome.kind === 'cancelled'), undo: done > 0 ? job.id : null };
}

/** An undo that ended: what was put back (`verb`, `Restored` by default), and what was left because it had changed since. */
export function restoredOf(job: JobView, result: JobResult, verb = 'Restored'): UndoReport {
  const { outcome } = result;
  if (outcome.kind === 'failed') {
    return { text: outcome.error.message, error: true, undo: null };
  }

  const done = doneOf(job, result);
  const failures = failedOf(result, verb === 'Restored' ? 'restored' : 'undone');
  const skipped = result.skipped > 0 ? `${verb} ${done}, skipped ${result.skipped}.` : `${verb} ${items(done)}.`;
  const text = outcome.kind === 'cancelled' ? `The undo was cancelled. ${skipped}` : skipped;
  return { text: `${text}${failures}`, error: failures !== '', undo: null };
}

/**
 * Tells how the jobs of one kind and their undos went. A job of that kind that ends is the job or the undo of one; which,
 * is known only from the answer to the undo that started it, and a short job may end before that answer arrives: the jobs
 * that end meanwhile wait for it.
 */
export class UndoReports {
  readonly #of: Undoable;
  readonly #report: (report: UndoReport) => void;
  /** The undo jobs asked for, and the job each takes back. */
  readonly #undone = new Map<JobId, JobId>();
  #asking = false;
  #waiting: { job: JobView; result: JobResult }[] = [];

  constructor(of: Undoable, report: (report: UndoReport) => void) {
    this.#of = of;
    this.#report = report;
  }

  /** A job ended; only those of this kind are told. */
  ended(job: JobView, result: JobResult) {
    if (job.kind !== this.#of.kind) {
      return;
    }

    if (this.#asking) {
      this.#waiting.push({ job, result });
    } else {
      this.#tell(job, result);
    }
  }

  /** Queues the undo of `job`; a refusal rejects and the job can be undone again. */
  async undo(ipc: Transport, job: JobId): Promise<void> {
    this.#asking = true;
    try {
      this.#undone.set(await ipc.invoke(this.#of.command, { job }), job);
    } finally {
      this.#asking = false;
      for (const waiting of this.#waiting.splice(0)) {
        this.#tell(waiting.job, waiting.result);
      }
    }
  }

  #tell(job: JobView, result: JobResult) {
    const original = this.#undone.get(job.id);
    if (original === undefined) {
      const report = this.#of.done(job, result);
      this.#report(this.#of.offers?.(job.id) === false ? { ...report, undo: null } : report);
      return;
    }

    const report = this.#of.undone(job, result);
    const stopped = result.outcome.kind !== 'done';
    this.#report(stopped && this.#of.again === true ? { ...report, undo: original } : report);
  }
}
