import { SvelteMap } from 'svelte/reactivity';

import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { type Decoded, parseDecoded } from '$lib/viewer/decoded';

import { THUMB_SIZE, type ThumbTarget } from './thumbs';

/** How many thumbnails are kept (a failed one counts too): the least recently asked for go first. */
const MAX_KEPT = 500;
/** How many requests are out at once; the backend decodes few pictures at a time and refuses the rest as busy. */
const MAX_IN_FLIGHT = 3;
/** How long a refusal as busy holds the queue back before the next try. */
const BUSY_WAIT_MS = 300;

export interface ThumbOptions {
  size?: number;
  max?: number;
  parallel?: number;
}

/**
 * The thumbnails of one pane. The pane says which pictures it wants ({@link want}, the ones in view first); they are
 * asked for a few at a time, the ones that scrolled out of view before their turn are dropped, and an answer is kept in
 * a bounded cache until it is the least recently wanted. A picture the backend cannot make a thumbnail of is kept as
 * such, so it is not asked for again. The pixels are drawn on canvases that come and go with the tiles, so nothing here
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
  /** Set while the queue waits after a refusal as busy. */
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
    this.#pump();
  }

  #pump() {
    if (this.#wait !== undefined) {
      return;
    }

    for (const target of this.#queue) {
      if (this.#flying.size >= this.#parallel) {
        return;
      }

      if (!this.#flying.has(target.key)) {
        void this.#request(target);
      }
    }
  }

  async #request(target: ThumbTarget) {
    this.#flying.add(target.key);
    const answer = await this.#load(target);
    this.#flying.delete(target.key);

    if (answer === 'busy') {
      this.#holdBack();
    } else {
      // Answered: it leaves the queue, so that a picture the cache has to drop again is not asked for in a loop.
      this.#queue = this.#queue.filter((queued) => queued.key !== target.key);
      this.#keep(target.key, answer);
    }

    this.#pump();
  }

  /** The picture, `null` when the backend cannot make a thumbnail of it, `'busy'` when it has no thread to spare for it now. */
  async #load({ folder, name }: ThumbTarget): Promise<Decoded | null | 'busy'> {
    try {
      return parseDecoded(await this.#ipc.invoke('thumb_get', { path: folder, name, size: this.size }));
    } catch (error) {
      return IpcFailure.from(error).code === 'busy' ? 'busy' : null;
    }
  }

  /** Waits a moment before asking again: the backend is full, and what it was asked is still queued. */
  #holdBack() {
    this.#wait ??= setTimeout(() => {
      this.#wait = undefined;
      this.#pump();
    }, BUSY_WAIT_MS);
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
