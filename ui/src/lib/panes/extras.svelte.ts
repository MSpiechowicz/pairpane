import { SvelteMap } from 'svelte/reactivity';

import type { Dimensions } from '$lib/ipc/bindings/Dimensions';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { type Cell, type Checksum, checksumCell, dimensionsCell, type ExtraTarget, type ExtraWants, pixelsOf } from './extras';
import type { ExtraColumn } from './order';

/** How many names one request for dimensions holds at the most (the backend refuses more). */
export const MAX_NAMES = 512;
/** How many values of each column are kept (a failed one counts too): the least recently wanted go first. */
const MAX_KEPT = 2000;
/** How many requests of a kind are out at once. The checksum of a big file takes a while, and the backend is busy with few. */
const DIMENSION_BATCHES = 1;
const CHECKSUMS = 2;
/**
 * How long a kind of request waits after the backend said it is busy: the first time, then twice as long each time it
 * says so again, up to {@link BUSY_WAIT_MAX_MS}; an answer that is not busy starts over. What is out is let be.
 */
const BUSY_WAIT_MS = 300;
const BUSY_WAIT_MAX_MS = 5_000;

export interface ExtraOptions {
  max?: number;
  checksums?: number;
}

/** A cache of a bounded size: when it is full the least recently wanted value goes. Reactive, so a cell shows its value on arrival. */
class Kept<T> {
  readonly #values = new SvelteMap<string, T>();
  /** The keys of `#values`, the least recently wanted first. */
  readonly #recent = new Map<string, true>();
  readonly #max: number;

  constructor(max: number) {
    this.#max = max;
  }

  get(key: string): T | undefined {
    return this.#values.get(key);
  }

  /** Whether `key` is kept; if it is, it is now the most recently wanted. Not reactive. */
  wanted(key: string): boolean {
    if (!this.#recent.has(key)) {
      return false;
    }

    this.#touch(key);
    return true;
  }

  set(key: string, value: T) {
    this.#values.set(key, value);
    this.#touch(key);

    for (const oldest of this.#recent.keys()) {
      if (this.#recent.size <= this.#max) {
        break;
      }

      this.#recent.delete(oldest);
      this.#values.delete(oldest);
    }
  }

  #touch(key: string) {
    this.#recent.delete(key);
    this.#recent.set(key, true);
  }
}

/** One kind of request: how many may be out at once, and the wait after the backend said it is busy. */
class Lane {
  /** The keys of the files being asked about. */
  readonly flying = new Set<string>();
  readonly #parallel: number;
  #holdUntil = 0;
  #wait = BUSY_WAIT_MS;

  constructor(parallel: number) {
    this.#parallel = parallel;
  }

  get free(): boolean {
    return this.flying.size < this.#parallel;
  }

  /** How long the lane still waits, `0` when requests may go. */
  waits(now: number): number {
    return Math.max(0, this.#holdUntil - now);
  }

  /** The backend is full: nothing new for a while, a longer one each time. */
  busy(now: number) {
    this.#holdUntil = Math.max(this.#holdUntil, now + this.#wait);
    this.#wait = Math.min(this.#wait * 2, BUSY_WAIT_MAX_MS);
  }

  /**
   * An answer that is not busy: the next wait starts over, unless the lane is still waiting (that answer is of a request
   * sent before the backend said it is busy, and says nothing of how full it is now).
   */
  answered(now: number) {
    if (now >= this.#holdUntil) {
      this.#wait = BUSY_WAIT_MS;
    }
  }
}

/**
 * The values of the extra columns of one pane: the size of the pictures in pixels, and the SHA-256 of the files. The pane
 * says what it wants ({@link want}, the rows in view first, only for the columns that are shown); the sizes of the
 * pictures of a folder are asked for in one request (up to {@link MAX_NAMES} names), the checksums one file at a time and
 * two at once at the most, which waits when the backend is busy, longer each time. What was queued and is no longer
 * wanted is dropped, and an answer is kept in a bounded cache until it is the least recently wanted. A file that has
 * no checksum (too big) or no dimensions is kept as such, so it is not asked for again until it changes. Nothing is
 * asked for inside an archive.
 */
export class Extras {
  readonly #dimensions: Kept<Dimensions | null>;
  readonly #checksums: Kept<Checksum>;
  readonly #dimensionLane = new Lane(DIMENSION_BATCHES);
  readonly #checksumLane: Lane;
  /** What is asked for, in the order to ask: what the pane wants that is not kept yet. */
  #dimensionQueue: readonly ExtraTarget[] = [];
  #checksumQueue: readonly ExtraTarget[] = [];
  /** Set while a lane waits for its time. */
  #wait: ReturnType<typeof setTimeout> | undefined;

  readonly #ipc: Transport;

  constructor(ipc: Transport, { max = MAX_KEPT, checksums = CHECKSUMS }: ExtraOptions = {}) {
    this.#ipc = ipc;
    this.#dimensions = new Kept(max);
    this.#checksums = new Kept(max);
    this.#checksumLane = new Lane(checksums);
  }

  /** What the cells of `target` show: nothing for a value that has not arrived (or does not exist for it). */
  cells(target: ExtraTarget | undefined): Record<ExtraColumn, Cell> {
    return {
      dimensions: dimensionsCell(target, target && this.#dimensions.get(target.key)),
      checksum: checksumCell(target, target && this.#checksums.get(target.key)),
    };
  }

  /** What a sort by `column` goes by for `target`; `undefined` while that is not known (and for a file with none). */
  sortKey(column: ExtraColumn, target: ExtraTarget | undefined): number | string | undefined {
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
  want({ dimensions, checksums }: ExtraWants) {
    this.#dimensionQueue = dimensions.filter((target) => !this.#dimensions.wanted(target.key));
    this.#checksumQueue = checksums.filter((target) => !this.#checksums.wanted(target.key));
    this.#pump();
  }

  #pump() {
    const now = Date.now();
    const next = [this.#askDimensions(now), this.#askChecksums(now)].filter((time) => time !== undefined);

    // A lane that waits for its time: the queues are looked at again then.
    if (next.length > 0) {
      this.#wakeAt(Math.min(...next), now);
    }
  }

  /** Sends the next batch (one folder's pictures); returns when to look again if the lane is waiting with work to do. */
  #askDimensions(now: number): number | undefined {
    const lane = this.#dimensionLane;

    while (lane.free) {
      const pending = this.#dimensionQueue.filter((target) => !lane.flying.has(target.key));
      if (pending.length === 0) {
        return undefined;
      }

      if (lane.waits(now) > 0) {
        return now + lane.waits(now);
      }

      const folder = pending[0].folder;
      void this.#loadDimensions(pending.filter((target) => target.folder === folder).slice(0, MAX_NAMES));
    }

    return undefined;
  }

  /** Sends the checksums that can go; returns when to look again if the lane is waiting with work to do. */
  #askChecksums(now: number): number | undefined {
    const lane = this.#checksumLane;

    while (lane.free) {
      const next = this.#checksumQueue.find((target) => !lane.flying.has(target.key));
      if (next === undefined) {
        return undefined;
      }

      if (lane.waits(now) > 0) {
        return now + lane.waits(now);
      }

      void this.#loadChecksum(next);
    }

    return undefined;
  }

  /** Looks at the queues again at `time` (unless they are to be looked at already). */
  #wakeAt(time: number, now: number) {
    if (this.#wait === undefined) {
      this.#wait = setTimeout(() => {
        this.#wait = undefined;
        this.#pump();
      }, time - now);
    }
  }

  /** The dimensions of the pictures `batch` (all in one folder); an answer that cannot be used leaves them without. */
  async #loadDimensions(batch: readonly ExtraTarget[]) {
    const lane = this.#dimensionLane;
    for (const target of batch) {
      lane.flying.add(target.key);
    }

    let answer: unknown;
    try {
      answer = await this.#ipc.invoke('columns_dimensions', { path: batch[0].folder, names: batch.map((target) => target.name) });
    } catch (error) {
      answer = IpcFailure.from(error).code === 'busy' ? 'busy' : [];
    }

    for (const target of batch) {
      lane.flying.delete(target.key);
    }

    if (answer === 'busy') {
      lane.busy(Date.now());
    } else {
      lane.answered(Date.now());
      const found = Array.isArray(answer) ? (answer as (Dimensions | null)[]) : [];
      batch.forEach((target, index) => this.#dimensions.set(target.key, found[index] ?? null));
      // Answered: they leave the queue, so that a value the cache has to drop again is not asked for in a loop.
      const done = new Set(batch.map((target) => target.key));
      this.#dimensionQueue = this.#dimensionQueue.filter((target) => !done.has(target.key));
    }

    this.#pump();
  }

  /** The checksum of one file; one that cannot be had (too big, not readable) is kept as a reason. */
  async #loadChecksum(target: ExtraTarget) {
    const lane = this.#checksumLane;
    lane.flying.add(target.key);

    let answer: Checksum | 'busy';
    try {
      const hex: unknown = await this.#ipc.invoke('columns_checksum', { path: target.folder, name: target.name });
      answer = typeof hex === 'string' ? { hex } : { problem: 'The backend gave no checksum.' };
    } catch (error) {
      const failure = IpcFailure.from(error);
      answer = failure.code === 'busy' ? 'busy' : { problem: failure.message };
    }

    lane.flying.delete(target.key);

    if (answer === 'busy') {
      lane.busy(Date.now());
    } else {
      lane.answered(Date.now());
      this.#checksums.set(target.key, answer);
      this.#checksumQueue = this.#checksumQueue.filter((queued) => queued.key !== target.key);
    }

    this.#pump();
  }
}
