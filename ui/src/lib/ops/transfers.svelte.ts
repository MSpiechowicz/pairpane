import type { Answer } from '$lib/ipc/bindings/Answer';
import type { ConflictInfo } from '$lib/ipc/bindings/ConflictInfo';
import type { ControlAction } from '$lib/ipc/bindings/ControlAction';
import type { ItemError } from '$lib/ipc/bindings/ItemError';
import type { JobId } from '$lib/ipc/bindings/JobId';
import type { JobView } from '$lib/ipc/bindings/JobView';
import type { OpsEvent } from '$lib/ipc/bindings/OpsEvent';
import type { Outcome } from '$lib/ipc/bindings/Outcome';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

/** A job that ended cleanly stays this long (it is shown as finished), then fades out. */
export const LINGER_MS = 5000;
export const FADE_MS = 300;

/** How a job ended; kept on its row until the user dismisses it or it fades out. */
export interface JobResult {
  outcome: Outcome;
  skipped: number;
  errors: ItemError[];
  moreErrors: number;
}

/** A job as the panel shows it. Rows are replaced on every change, never changed in place. */
export interface JobRow extends JobView {
  result?: JobResult;
  /** The row is fading out and about to go. */
  leaving?: boolean;
}

/** A conflict that waits for an answer, and what Compare found out about it. */
export interface Question {
  info: ConflictInfo;
  compare?: 'running' | 'identical' | 'different';
}

/** What the chip shows: the jobs there are, and how far the running ones have come (bytes of all of them). */
export interface Overall {
  count: number;
  percent: number | undefined;
}

/** An answer the backend refused because the question is gone (answered elsewhere, the job ended) is not worth a retry. */
function isStale(error: unknown): boolean {
  const code = IpcFailure.from(error).code;
  return code === 'invalidInput' || code === 'notFound';
}

/** An ended job that the user has to look at: it failed, or some items did. */
function needsLook(result: JobResult): boolean {
  return result.outcome.kind === 'failed' || result.errors.length > 0 || result.moreErrors > 0;
}

/**
 * The copy and move jobs as the UI sees them: one subscription for the whole app, a row per job, and the
 * conflicts waiting for an answer, queued so that the dialog shows one at a time. The backend is the source of
 * truth; after a page reload the snapshot it sends on subscribe brings back the jobs and the open questions.
 */
export class Transfers {
  readonly #ipc: Transport;
  readonly #onEnded: ((job: JobView, result: JobResult) => void) | undefined;

  /** Plain collections (their rows are replaced, not changed); readers track `#revision`, bumped on every change. */
  readonly #jobs = new Map<JobId, JobRow>();
  #questions: Question[] = [];
  #revision = $state(0);
  readonly #timers = new Map<JobId, ReturnType<typeof setTimeout>>();

  readonly jobs = $derived.by(() => {
    void this.#revision;
    return [...this.#jobs.values()];
  });

  readonly overall = $derived.by((): Overall => {
    let done = 0;
    let total = 0;
    for (const job of this.jobs) {
      if (job.result === undefined) {
        done += job.doneBytes;
        total += job.totalBytes;
      }
    }

    return { count: this.jobs.length, percent: total > 0 ? Math.min(100, Math.floor((done / total) * 100)) : undefined };
  });

  /** The conflict to ask about now: the oldest one that waits. */
  readonly question = $derived.by(() => {
    void this.#revision;
    return this.#questions[0];
  });

  /** `onEnded` hears of every job that ends while the page is open, after its row shows the result. */
  constructor(ipc: Transport, onEnded?: (job: JobView, result: JobResult) => void) {
    this.#ipc = ipc;
    this.#onEnded = onEnded;
  }

  /** Starts listening. The backend answers with a snapshot, so this is also how a reloaded page catches up. */
  async subscribe(): Promise<void> {
    await this.#ipc.invoke('ops_subscribe', { onEvent: (event) => this.#apply(event) });
  }

  /** Pauses, resumes or cancels a job. A job that is gone by now has nothing left to control. */
  async control(job: JobId, action: ControlAction): Promise<void> {
    try {
      await this.#ipc.invoke('ops_control', { job, action });
    } catch {
      // The job ended meanwhile; its `ended` event has told or will tell the panel.
    }
  }

  /** Cancels a job from its conflict dialog: its questions are gone with it, the dialog does not wait for the end. */
  async cancelJob(job: JobId): Promise<void> {
    this.#questions = this.#questions.filter((question) => question.info.job !== job);
    this.#revision += 1;
    await this.control(job, 'cancel');
  }

  /**
   * Answers a question. A final answer takes the question off the queue at once (a double click must not answer
   * twice); if the backend could not take it, it comes back, unless it was stale. Compare is not final: the
   * question stays, with the outcome of the comparison once it arrives.
   */
  async answer(question: Question, answer: Answer, all: boolean): Promise<void> {
    const { job, question: number } = question.info;
    const compare = answer === 'compare';

    if (compare) {
      this.#markCompare(job, number, 'running');
    } else {
      this.#questions = this.#questions.filter((open) => !this.#is(open, job, number));
      this.#revision += 1;
    }

    try {
      await this.#ipc.invoke('ops_answer', { job, question: number, answer, all });
    } catch (error) {
      if (compare) {
        this.#markCompare(job, number, undefined);
      } else if (!isStale(error) && this.#jobs.get(job)?.result === undefined) {
        this.#questions = [question, ...this.#questions];
        this.#revision += 1;
      }
    }
  }

  /** Removes an ended job from the panel. */
  dismiss(id: JobId) {
    if (this.#jobs.get(id)?.result !== undefined) {
      this.#drop(id);
      this.#revision += 1;
    }
  }

  #apply(event: OpsEvent) {
    switch (event.kind) {
      case 'snapshot':
        this.#replace(event.jobs, event.questions);
        break;
      case 'added':
        this.#jobs.set(event.job.id, event.job);
        break;
      case 'phase':
        this.#patch(event.id, { phase: event.phase });
        break;
      case 'paused':
        this.#patch(event.id, { paused: event.paused });
        break;
      case 'progress':
        this.#patch(event.id, {
          doneBytes: event.doneBytes,
          totalBytes: event.totalBytes,
          doneFiles: event.doneFiles,
          totalFiles: event.totalFiles,
          current: event.current,
          bytesPerSec: event.bytesPerSec,
          etaSecs: event.etaSecs,
        });
        break;
      case 'conflict':
        this.#ask(event.conflict);
        break;
      case 'compared':
        this.#markCompare(event.id, event.question, event.identical ? 'identical' : 'different');
        break;
      case 'ended':
        this.#end(event.id, {
          outcome: event.outcome,
          skipped: event.skipped,
          errors: event.errors,
          moreErrors: event.moreErrors,
        });
        break;
    }

    this.#revision += 1;
  }

  #replace(jobs: JobView[], questions: ConflictInfo[]) {
    for (const id of [...this.#jobs.keys()]) {
      this.#drop(id);
    }

    for (const job of jobs) {
      this.#jobs.set(job.id, job);
    }

    this.#questions = questions.map((info) => ({ info }));
  }

  /** A change of a job that is running; one that ended (a late progress) or is unknown is not touched. */
  #patch(id: JobId, change: Partial<JobView>) {
    const job = this.#jobs.get(id);
    if (job !== undefined && job.result === undefined) {
      this.#jobs.set(id, { ...job, ...change });
    }
  }

  #is(open: Question, job: JobId, number: number): boolean {
    return open.info.job === job && open.info.question === number;
  }

  /** A conflict of a job that is running; one of an unknown or ended job has nobody to answer it. */
  #ask(info: ConflictInfo) {
    const job = this.#jobs.get(info.job);
    if (job === undefined || job.result !== undefined) {
      return;
    }

    if (!this.#questions.some((open) => this.#is(open, info.job, info.question))) {
      this.#questions = [...this.#questions, { info }];
    }
  }

  #markCompare(job: JobId, number: number, compare: Question['compare']) {
    this.#questions = this.#questions.map((open) =>
      this.#is(open, job, number) ? { ...open, compare } : open,
    );
    this.#revision += 1;
  }

  #end(id: JobId, result: JobResult) {
    const job = this.#jobs.get(id);
    if (job === undefined) {
      return;
    }

    this.#questions = this.#questions.filter((open) => open.info.job !== id);
    this.#jobs.set(id, { ...job, result });

    if (!needsLook(result)) {
      this.#timers.set(id, setTimeout(() => this.#fade(id), LINGER_MS));
    }

    this.#onEnded?.(job, result);
  }

  #fade(id: JobId) {
    const job = this.#jobs.get(id);
    if (job === undefined) {
      return;
    }

    this.#jobs.set(id, { ...job, leaving: true });
    this.#timers.set(id, setTimeout(() => this.dismiss(id), FADE_MS));
    this.#revision += 1;
  }

  /** Forgets a job and its pending timer. */
  #drop(id: JobId) {
    clearTimeout(this.#timers.get(id));
    this.#timers.delete(id);
    this.#jobs.delete(id);
  }
}
