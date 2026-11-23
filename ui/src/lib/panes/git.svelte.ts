import type { GitStatus } from '$lib/ipc/bindings/GitStatus';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { type Cell, gitCell, type GitFolder, type GitPlace, severityOf } from './extras';
import { type Keyed, Lane, Wake } from './extrasLane';
import { Kept } from './kept.svelte';

/** How long what git said about a folder holds: an edit in an editor shows up by then, and a `git add` (which the listing does not see) too. */
export const GIT_TTL_MS = 10_000;
/** How many folders' answers are kept, and how many folders a pane may want at once (fewer, so what is wanted always fits). */
const MAX_KEPT = 256;
const MAX_WANTED = 100;
/** How long what is kept about a repository too big to read (or that took too long) holds, though it is a reload that asks soonest. */
const TOO_BIG_TTL_MS = 60_000;
/** How many requests are out at once: one, since the backend does git for two requests at a time in all, for every pane. */
const PARALLEL = 1;
/** How long a folder that could not be read for a passing reason waits before it is asked about again. */
const FAILED_WAIT_MS = 10_000;

export interface GitOptions {
  ttl?: number;
  parallel?: number;
}

/** What was said about a folder, and when (and how many reloads of the listing had happened by then). */
interface Answer {
  folder: GitFolder;
  at: number;
  reloads: number;
}

/** The marks of an answer by name. */
function folderOf(status: GitStatus): GitFolder {
  return status.repo ? { marks: new Map(status.marks.map(({ name, mark }) => [name, mark])) } : null;
}

/**
 * The git status of the folders a pane shows items of, for the Git column. A request covers one whole folder (its
 * direct children), so the pane says which folders hold the items in view ({@link want}), not which items. One request
 * is out at a time (the backend serves two for all the panes); the answers are kept per folder until the listing is reloaded (a change on disk) or
 * {@link GIT_TTL_MS} has passed, whichever is first, and the folder is asked about again then if still wanted (what
 * is kept shows until the new answer comes). A folder that is not in a repository is kept as such. A repository too big
 * to read (or that took too long) is kept as the reason, and is asked about again when the listing is reloaded, or after
 * a minute. A busy answer makes the lane wait, longer each time up to a few seconds, and go on at that pace for as long
 * as the folder is wanted (other panes take the backend's slots for a while, and no one is waited out for good);
 * another failure makes the folder wait a while.
 */
export class GitFolders {
  readonly #answers: Kept<Answer>;
  readonly #lane: Lane;
  readonly #ttl: number;
  readonly #wake = new Wake(() => {
    this.#queue = this.#stale(Date.now());
    this.#pump();
  });
  readonly #ipc: Transport;

  /** The folders the pane wants, the most wanted first, and the reloads of its listing when it said so. */
  #folders: readonly string[] = [];
  #reloads = 0;
  /** What is asked for, in the order to ask: the folders wanted whose answer is missing or out of date. */
  #queue: readonly Keyed[] = [];

  constructor(ipc: Transport, { ttl = GIT_TTL_MS, parallel = PARALLEL }: GitOptions = {}) {
    this.#ipc = ipc;
    this.#ttl = ttl;
    this.#answers = new Kept(MAX_KEPT);
    this.#lane = new Lane(parallel, { persistent: true });
  }

  /** What the Git cell of `place` shows: nothing for a clean item, outside a repository and until the answer arrives. */
  cell(place: GitPlace | undefined): Cell {
    return gitCell(place, place && this.#answers.get(place.folder)?.folder);
  }

  /** What a sort by the column goes by for `place`; see {@link severityOf}. */
  severity(place: GitPlace | undefined): number | undefined {
    return severityOf(place, place && this.#answers.get(place.folder)?.folder);
  }

  /**
   * Sets which folders the pane wants the status of now, the most wanted first, and how many times its listing has been
   * reloaded. What was queued and is not wanted any more is dropped; a request already out is let be.
   */
  want(folders: readonly string[], reloads: number) {
    this.#folders = folders.slice(0, MAX_WANTED);
    this.#reloads = reloads;

    const now = Date.now();
    this.#queue = this.#stale(now);
    this.#lane.retarget(this.#queue, now);
    this.#pump();
  }

  /** Whether what is kept about `folder` can be trusted still. */
  #fresh(folder: string, now: number): boolean {
    const kept = this.#answers.peek(folder);
    if (kept === undefined || kept.reloads !== this.#reloads) {
      return false;
    }

    return now - kept.at < this.#holds(kept.folder);
  }

  /** How long an answer holds: a repository too big to read stays so for longer, since asking again soon would only repeat the refusal. */
  #holds(folder: GitFolder): number {
    return tooBig(folder) ? Math.max(TOO_BIG_TTL_MS, this.#ttl) : this.#ttl;
  }

  /** The wanted folders that are not fresh, in the order wanted. */
  #stale(now: number): Keyed[] {
    return this.#folders.filter((folder) => !this.#fresh(folder, now)).map((key) => ({ key }));
  }

  #pump() {
    const now = Date.now();
    const times = [this.#ask(now), this.#nextExpiry(now)].filter((time) => time !== undefined);

    if (times.length > 0) {
      this.#wake.at(Math.min(...times), now);
    } else {
      // Nothing is waited for (no folder is wanted, or none is left to ask or to expire): no look is left set.
      this.#wake.clear();
    }
  }

  /** When the first answer kept for a wanted folder runs out; `undefined` if none will. */
  #nextExpiry(now: number): number | undefined {
    const times = this.#folders.flatMap((folder) => {
      const kept = this.#answers.peek(folder);
      return kept !== undefined && this.#fresh(folder, now) ? [kept.at + this.#holds(kept.folder)] : [];
    });

    return times.length > 0 ? Math.min(...times) : undefined;
  }

  /** Sends the requests that can go; returns when to look again if the lane is waiting with work to do. */
  #ask(now: number): number | undefined {
    return this.#lane.drain(this.#queue, now, (due) => void this.#load(due[0].key));
  }

  /**
   * The status of `folder`. A repository too big to read, or that took too long (the backend's `invalidInput`, and no
   * other kind), is kept as the reason; any other failure than a busy one is not kept, and the folder waits a while.
   */
  async #load(folder: string) {
    const lane = this.#lane;
    const reloads = this.#reloads;
    const asked = [{ key: folder }];
    lane.start(asked);

    let answer: GitFolder | 'busy' | 'failed';
    try {
      const status: unknown = await this.#ipc.invoke('columns_git', { path: folder });
      answer = isStatus(status) ? folderOf(status) : 'failed';
    } catch (error) {
      const failure = IpcFailure.from(error);
      if (failure.code === 'busy') {
        answer = 'busy';
      } else {
        answer = failure.code === 'invalidInput' ? { problem: failure.message } : 'failed';
      }
    }

    lane.finish(asked);

    const now = Date.now();
    if (answer === 'busy') {
      lane.busy(now, this.#queue);
    } else if (answer === 'failed') {
      lane.answered(now);
      lane.delay(folder, now + FAILED_WAIT_MS);
    } else {
      lane.answered(now);
      this.#answers.set(folder, { folder: answer, at: now, reloads });
      // Answered: it leaves the queue, so that an answer the cache has to drop again is not asked for in a loop. An answer
      // of before a reload stays: it is out of date already.
      if (reloads === this.#reloads) {
        this.#queue = this.#queue.filter((queued) => queued.key !== folder);
      }
    }

    this.#pump();
  }
}

/** Whether `folder` is the reason a repository cannot be read, not its status. */
function tooBig(folder: GitFolder): boolean {
  return folder !== null && 'problem' in folder;
}

/** Whether what the backend answered is a status (an answer of another shape is a failure, not "no repository"). */
function isStatus(value: unknown): value is GitStatus {
  return typeof value === 'object' && value !== null && 'repo' in value && 'marks' in value && Array.isArray((value as GitStatus).marks);
}
