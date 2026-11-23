import { SvelteMap } from 'svelte/reactivity';

import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { type Decoded, parseDecoded } from '$lib/viewer/decoded';

import { THUMB_SIZE, type ThumbTarget } from './thumbs';

/** How many thumbnails are kept (a failed one counts too): the least recently asked for go first. */
const MAX_KEPT = 500;
/** How many requests are out at once; the backend decodes few pictures at a time and refuses the rest as busy. */
const MAX_IN_FLIGHT = 3;
/**
 * How long the pane asks for nothing new after the backend said it is busy: the first time, then twice as long each
 * time it says so again, up to {@link BUSY_WAIT_MAX_MS}; an answer that is not busy starts over. What is out is let be.
 */
const BUSY_WAIT_MS = 300;
const BUSY_WAIT_MAX_MS = 5_000;
/** How long a picture that could not be read for a passing reason (a read cut short, a file being written) waits. */
const FAILED_WAIT_MS = 10_000;

export interface ThumbOptions {
  size?: number;
  max?: number;
  parallel?: number;
}

/**
 * The thumbnails of one pane. The pane says which pictures it wants ({@link want}, the ones in view first); they are
 * asked for a few at a time, the ones that scrolled out of view before their turn are dropped, and an answer is kept in
 * a bounded cache until it is the least recently wanted. A picture the backend cannot make a thumbnail of (not a
 * picture, or one it cannot decode) is kept as such, so it is not asked for again; one not read for a passing reason
 * waits a while (that picture alone) and is asked for again if it is still wanted. When the backend is busy, the pane
 * waits before it asks for anything new, longer each time, so that a full backend is not asked again and again. The pixels are drawn on canvases that come and go with the tiles, so nothing here
 * has to be released.
 */
export class Thumbs {
  /** What the backend answered, by key; `null`: no thumbnail for it. Reactive, so a tile shows its picture on arrival. */
  readonly #pictures = new SvelteMap<string, Decoded | null>();
  /** The keys of `#pictures`, the least recently wanted first. */
  readonly #recent = new Map<string, true>();
  /** What is asked for, in the order to ask: the pictures the pane wants that are not here yet. */
  #queue: readonly ThumbTarget[] = [];
  readonly #flying = new Set<string>();
  /** Pictures not to be asked for before a time (not read for a passing reason), by key. */
  readonly #notBefore = new Map<string, number>();
  /** Nothing new is asked for before this time (the backend said it is busy), and how long the next wait will be. */
  #holdUntil = 0;
  #busyWait = BUSY_WAIT_MS;
  /** Set while a picture waits for its time. */
  #wait: ReturnType<typeof setTimeout> | undefined;

  readonly #ipc: Transport;
  readonly size: number;
  readonly #max: number;
  readonly #parallel: number;

  constructor(ipc: Transport, { size = THUMB_SIZE, max = MAX_KEPT, parallel = MAX_IN_FLIGHT }: ThumbOptions = {}) {
    this.#ipc = ipc;
    this.size = size;
    this.#max = max;
    this.#parallel = parallel;
  }

  /** The thumbnail of `target` once it has arrived; `undefined` while it is awaited and when there is none (the tile shows an icon). */
  get(target: ThumbTarget | undefined): Decoded | undefined {
    return target === undefined ? undefined : (this.#pictures.get(target.key) ?? undefined);
  }

  /**
   * Sets what the pane wants now, the most wanted first. What it wanted before and has not been asked for yet is
   * dropped; a request already out is let be, and its answer kept. What is kept already is not asked for again.
   */
  want(targets: readonly ThumbTarget[]) {
    const missing: ThumbTarget[] = [];

    for (const target of targets) {
      if (this.#recent.has(target.key)) {
        this.#touch(target.key);
      } else {
        missing.push(target);
      }
    }

    this.#queue = missing;

    // A wait that is over, or of a picture no longer wanted, is forgotten (asked again if it comes back).
    const now = Date.now();
    const queued = new Set(missing.map((target) => target.key));
    for (const [key, after] of this.#notBefore) {
      if (after <= now || !queued.has(key)) {
        this.#notBefore.delete(key);
      }
    }

    this.#pump();
  }

  #pump() {
    const now = Date.now();
    let next: number | undefined;

    if (now < this.#holdUntil) {
      this.#wakeAt(this.#holdUntil, now);
      return;
    }

    for (const target of this.#queue) {
      if (this.#flying.size >= this.#parallel) {
        return;
      }

      const after = this.#notBefore.get(target.key);
      if (after !== undefined && after > now) {
        next = Math.min(next ?? after, after);
      } else if (!this.#flying.has(target.key)) {
        this.#notBefore.delete(target.key);
        void this.#request(target);
      }
    }

    // Some wanted pictures wait for their time: the queue is looked at again then.
    if (next !== undefined) {
      this.#wakeAt(next, now);
    }
  }

  /**
   * An answer that is not busy: the next wait starts over, unless the pane is still waiting (that answer is of a request
   * sent before the backend said it is busy, and says nothing of how full it is now).
   */
  #settle() {
    if (Date.now() >= this.#holdUntil) {
      this.#busyWait = BUSY_WAIT_MS;
    }
  }

  /** Looks at the queue again at `time` (unless it is to be looked at already). */
  #wakeAt(time: number, now: number) {
    if (this.#wait === undefined) {
      this.#wait = setTimeout(() => {
        this.#wait = undefined;
        this.#pump();
      }, time - now);
    }
  }

  async #request(target: ThumbTarget) {
    this.#flying.add(target.key);
    const answer = await this.#load(target);
    this.#flying.delete(target.key);

    if (answer === 'busy') {
      // The backend is full: the pane waits before it asks for more (the picture stays queued).
      this.#holdUntil = Math.max(this.#holdUntil, Date.now() + this.#busyWait);
      this.#busyWait = Math.min(this.#busyWait * 2, BUSY_WAIT_MAX_MS);
    } else if (answer === 'failed') {
      this.#settle();
      this.#notBefore.set(target.key, Date.now() + FAILED_WAIT_MS);
    } else {
      this.#settle();
      // Answered: it leaves the queue, so that a picture the cache has to drop again is not asked for in a loop.
      this.#queue = this.#queue.filter((queued) => queued.key !== target.key);
      this.#keep(target.key, answer);
    }

    this.#pump();
  }

  /**
   * The picture; `null` when the backend cannot make a thumbnail of it (not a picture, or one it cannot decode, or an
   * answer that is not one), `'busy'` when it has no thread to spare for it now, `'failed'` when it could not be read
   * for a reason that may pass.
   */
  async #load({ folder, name }: ThumbTarget): Promise<Decoded | null | 'busy' | 'failed'> {
    let answer: ArrayBuffer;
    try {
      answer = await this.#ipc.invoke('thumb_get', { path: folder, name, size: this.size });
    } catch (error) {
      const code = IpcFailure.from(error).code;
      if (code === 'busy') {
        return 'busy';
      }
      return code === 'unsupported' || code === 'invalidInput' ? null : 'failed';
    }

    try {
      return parseDecoded(answer);
    } catch {
      return null;
    }
  }

  #keep(key: string, picture: Decoded | null) {
    this.#pictures.set(key, picture);
    this.#touch(key);

    for (const oldest of this.#recent.keys()) {
      if (this.#recent.size <= this.#max) {
        break;
      }

      this.#recent.delete(oldest);
      this.#pictures.delete(oldest);
    }
  }

  /** Makes `key` the most recently wanted. */
  #touch(key: string) {
    this.#recent.delete(key);
    this.#recent.set(key, true);
  }
}
