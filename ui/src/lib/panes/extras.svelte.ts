import type { Dimensions } from '$lib/ipc/bindings/Dimensions';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import {
  type Cell,
  type Checksum,
  checksumCell,
  dimensionsCell,
  type ExtraTarget,
  type ExtraWants,
  type GitPlace,
  pixelsOf,
} from './extras';
import { Lane, Wake } from './extrasLane';
import { GitFolders } from './git.svelte';
import { Kept } from './kept.svelte';
import type { ExtraColumn } from './order';

/** How many names one request for dimensions holds at the most (the backend refuses more). */
export const MAX_NAMES = 512;
/** How many values of each column are kept (a failed one counts too): the least recently wanted go first. */
const MAX_KEPT = 2000;
/** How many requests of a kind are out at once. The backend has four threads for them all, and the checksum of a big file takes a while. */
const DIMENSION_BATCHES = 2;
const CHECKSUMS = 2;
/** How long a file that could not be read for a passing reason waits before it is asked about again (if it is still wanted). */
const FAILED_WAIT_MS = 10_000;

export interface ExtraOptions {
  max?: number;
  checksums?: number;
  /** How long what git said about a folder holds, in milliseconds. */
  gitTtl?: number;
}

/**
 * The values of the extra columns of one pane: the size of the pictures in pixels, and the SHA-256 of the files. The pane
 * says what it wants ({@link want}, the rows in view first, only for the columns that are shown); the sizes of the
 * pictures of a folder are asked for in one request (up to {@link MAX_NAMES} names), the checksums one file at a time and
 * two at once at the most, which waits when the backend is busy, longer each time, and stops asking after too many busy
 * answers in a row, until the pane wants something else. What was queued and is no longer wanted is dropped, and an
 * answer is kept in a bounded cache until it is the least recently wanted. A file that has no checksum (too big, not on
 * this computer) or no dimensions is kept as such, so it is not asked for again until it changes. A request that fails
 * for a passing reason is not kept: its files wait a while and are asked for again if still wanted. Nothing is asked for
 * inside an archive. The git status of the folders in view is {@link git}'s (it asks a folder at a time).
 */
export class Extras {
  /** The git status of the folders the pane shows items of. */
  readonly git: GitFolders;

  readonly #dimensions: Kept<Dimensions | null>;
  readonly #checksums: Kept<Checksum>;
  readonly #dimensionLane = new Lane(DIMENSION_BATCHES);
  readonly #checksumLane: Lane;
  /** What is asked for, in the order to ask: what the pane wants that is not kept yet. */
  #dimensionQueue: readonly ExtraTarget[] = [];
  #checksumQueue: readonly ExtraTarget[] = [];
  /** Looks at the queues again when a lane that waits has its time. */
  readonly #wake = new Wake(() => this.#pump());

  readonly #ipc: Transport;

  constructor(ipc: Transport, { max = MAX_KEPT, checksums = CHECKSUMS, gitTtl }: ExtraOptions = {}) {
    this.#ipc = ipc;
    this.git = new GitFolders(ipc, { ttl: gitTtl });
    this.#dimensions = new Kept(max);
    this.#checksums = new Kept(max);
    this.#checksumLane = new Lane(checksums);
  }

  /**
   * What the cells of an item show: nothing for a value that has not arrived (or does not exist for it). `target` is the
   * item as a file, `place` as anything that has a git status.
   */
  cells(target: ExtraTarget | undefined, place?: GitPlace): Record<ExtraColumn, Cell> {
    return {
      dimensions: dimensionsCell(target, target && this.#dimensions.get(target.key)),
      checksum: checksumCell(target, target && this.#checksums.get(target.key)),
      git: this.git.cell(place),
    };
  }

  /** What a sort by `column` goes by for an item (see {@link cells}); `undefined` while that is not known (and for an item with none). */
  sortKey(column: ExtraColumn, target: ExtraTarget | undefined, place?: GitPlace): number | string | undefined {
    if (column === 'git') {
      return this.git.severity(place);
    }

    if (target === undefined) {
      return undefined;
    }

    if (column === 'dimensions') {
      return pixelsOf(this.#dimensions.get(target.key));
    }

    const checksum = this.#checksums.get(target.key);
    return checksum !== undefined && 'hex' in checksum ? checksum.hex : undefined;
  }

  /**
   * Sets what the pane wants now, the most wanted first. What it wanted before and has not been asked for yet is
   * dropped; a request already out is let be, and its answer kept. What is kept already is not asked for again.
   */
  want({ dimensions, checksums, git, reloads }: ExtraWants) {
    const now = Date.now();

    this.git.want(git, reloads);
    this.#dimensionQueue = dimensions.filter((target) => !this.#dimensions.wanted(target.key));
    this.#checksumQueue = checksums.filter((target) => !this.#checksums.wanted(target.key));
    this.#dimensionLane.retarget(this.#dimensionQueue, now);
    this.#checksumLane.retarget(this.#checksumQueue, now);
    this.#pump();
  }

  #pump() {
    const now = Date.now();
    const next = [this.#askDimensions(now), this.#askChecksums(now)].filter((time) => time !== undefined);

    // A lane that waits for its time: the queues are looked at again then.
    if (next.length > 0) {
      this.#wake.at(Math.min(...next), now);
    } else {
      this.#wake.clear();
    }
  }

  /** Sends the next batches (one folder's pictures each); returns when to look again if the lane is waiting with work to do. */
  #askDimensions(now: number): number | undefined {
    return this.#dimensionLane.drain(this.#dimensionQueue, now, (due) => {
      const folder = due[0].folder;
      void this.#loadDimensions(due.filter((target) => target.folder === folder).slice(0, MAX_NAMES));
    });
  }

  /** Sends the checksums that can go; returns when to look again if the lane is waiting with work to do. */
  #askChecksums(now: number): number | undefined {
    return this.#checksumLane.drain(this.#checksumQueue, now, (due) => void this.#loadChecksum(due[0]));
  }

  /**
   * The dimensions of the pictures `batch` (all in one folder). An answer for the whole call is kept for each name (the
   * backend says `null` for a name it has none of); a call that failed keeps nothing, and the names wait a while.
   */
  async #loadDimensions(batch: readonly ExtraTarget[]) {
    const lane = this.#dimensionLane;
    lane.start(batch);

    let answer: (Dimensions | null)[] | 'busy' | 'failed';
    try {
      const found: unknown = await this.#ipc.invoke('columns_dimensions', { path: batch[0].folder, names: batch.map((target) => target.name) });
      answer = Array.isArray(found) ? (found as (Dimensions | null)[]) : 'failed';
    } catch (error) {
      answer = IpcFailure.from(error).code === 'busy' ? 'busy' : 'failed';
    }

    lane.finish(batch);

    const now = Date.now();
    if (answer === 'busy') {
      lane.busy(now, this.#dimensionQueue);
    } else if (answer === 'failed') {
      lane.answered(now);
      batch.forEach((target) => lane.delay(target.key, now + FAILED_WAIT_MS));
    } else {
      lane.answered(now);
      batch.forEach((target, index) => this.#dimensions.set(target.key, answer[index] ?? null));
      // Answered: they leave the queue, so that a value the cache has to drop again is not asked for in a loop.
      const done = new Set(batch.map((target) => target.key));
      this.#dimensionQueue = this.#dimensionQueue.filter((target) => !done.has(target.key));
    }

    this.#pump();
  }

  /**
   * The checksum of one file. One that cannot be had for good (too big, not on this computer) is kept as a reason; any
   * other failure is not kept, and the file waits a while.
   */
  async #loadChecksum(target: ExtraTarget) {
    const lane = this.#checksumLane;
    lane.start([target]);

    let answer: Checksum | 'busy' | 'failed';
    try {
      const hex: unknown = await this.#ipc.invoke('columns_checksum', { path: target.folder, name: target.name });
      answer = typeof hex === 'string' ? { hex } : 'failed';
    } catch (error) {
      const failure = IpcFailure.from(error);
      if (failure.code === 'busy') {
        answer = 'busy';
      } else {
        answer = failure.code === 'invalidInput' ? { problem: failure.message } : 'failed';
      }
    }

    lane.finish([target]);

    const now = Date.now();
    if (answer === 'busy') {
      lane.busy(now, this.#checksumQueue);
    } else if (answer === 'failed') {
      lane.answered(now);
      lane.delay(target.key, now + FAILED_WAIT_MS);
    } else {
      lane.answered(now);
      this.#checksums.set(target.key, answer);
      this.#checksumQueue = this.#checksumQueue.filter((queued) => queued.key !== target.key);
    }

    this.#pump();
  }
}
