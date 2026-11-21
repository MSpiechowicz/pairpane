import type { SearchHit } from '$lib/ipc/bindings/SearchHit';
import type { SearchNeedle } from '$lib/ipc/bindings/SearchNeedle';
import { IpcFailure } from '$lib/ipc/transport';

import { parseHex } from './hex';
import type { ViewSession } from './session.svelte';

/** Searches closer together than this are joined into the last one asked for, so holding Enter cannot fill the backend. */
export const SEARCH_GAP_MS = 150;

export type SearchStatus = 'idle' | 'searching' | 'none' | 'failed';

/**
 * The search box of the viewer: next and previous match of a text (with or without case) or of bytes, from the
 * previous match or else from the top of the view. A search is asked of the backend; the answer of one that a newer
 * search or a cancel has replaced is dropped (the backend ends such a search with the answer for "not found").
 */
export class ViewSearch {
  text = $state('');
  caseSensitive = $state(false);
  hex = $state(false);
  status = $state<SearchStatus>('idle');
  /** The bytes scanned so far by the search that runs, for a long one. */
  progress = $state(0);
  failure = $state<string | undefined>();
  /** The latest match; the views show it and the next search goes on from it. */
  hit = $state.raw<SearchHit | undefined>();

  readonly #session: ViewSession;
  #ticket = 0;
  #started = Number.NEGATIVE_INFINITY;
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(session: ViewSession) {
    this.#session = session;
  }

  /** Hex was asked for and the text is not hex. */
  get invalid(): boolean {
    return this.hex && this.text.trim() !== '' && parseHex(this.text) === undefined;
  }

  get searching(): boolean {
    return this.status === 'searching';
  }

  /** What to look for; `undefined` for an empty or invalid box. */
  #needle(): SearchNeedle | undefined {
    if (this.hex) {
      const bytes = parseHex(this.text);

      return bytes === undefined || bytes.length === 0 ? undefined : { kind: 'hex', bytes };
    }

    return this.text === '' ? undefined : { kind: 'text', text: this.text, caseSensitive: this.caseSensitive };
  }

  /**
   * The needle changed: the next search starts from the view again, not from the match of the old one; a search for
   * the old needle that still runs is stopped, so that its answer cannot become the match of the new one.
   */
  edited() {
    this.hit = undefined;
    this.#stop();
    this.status = 'idle';
  }

  /** Finds the next (or previous) match; a search asked right after another waits for the gap to pass. */
  run(backward: boolean) {
    clearTimeout(this.#timer);
    const wait = this.#started + SEARCH_GAP_MS - Date.now();

    if (wait > 0) {
      this.#timer = setTimeout(() => void this.#start(backward), wait);
    } else {
      void this.#start(backward);
    }
  }

  async #start(backward: boolean) {
    const session = this.#session;
    const needle = this.#needle();
    if (needle === undefined || session.id === undefined) {
      return;
    }

    this.#started = Date.now();
    this.#ticket += 1;
    const ticket = this.#ticket;
    this.status = 'searching';
    this.progress = 0;

    // Next goes on after the match, previous before it; with no match yet, from the top of the view.
    const from = this.hit === undefined ? session.position : backward ? this.hit.offset : this.hit.offset + this.hit.len;
    const query = { needle, encoding: session.encoding, from, backward };

    try {
      const found = await session.ipc.invoke('view_search', {
        session: session.id,
        query,
        onProgress: (scanned) => {
          if (ticket === this.#ticket) {
            this.progress = scanned;
          }
        },
      });
      if (ticket !== this.#ticket) {
        return;
      }

      this.status = found === null ? 'none' : 'idle';
      if (found !== null) {
        this.hit = found;
      }
    } catch (error) {
      if (ticket === this.#ticket) {
        this.failure = IpcFailure.from(error).message;
        this.status = 'failed';
      }
    }
  }

  /** Stops the search that runs; its answer, when it comes, is dropped. */
  cancel() {
    clearTimeout(this.#timer);
    this.#stop();
  }

  #stop() {
    this.#ticket += 1;

    if (this.status === 'searching') {
      this.status = 'idle';
      const session = this.#session;
      if (session.id !== undefined) {
        session.ipc.invoke('view_search_cancel', { session: session.id }).catch(() => {});
      }
    }
  }

  /** The viewer is going away: nothing is delivered after this (the backend cancels with the session). */
  dispose() {
    clearTimeout(this.#timer);
    this.#ticket += 1;
  }
}
