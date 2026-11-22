import { IpcFailure } from '$lib/ipc/transport';

import { readBytes } from './bytes';
import { lastTop, ROW_BYTES, rowStart, windowBytes } from './hex';
import { moveOf, Wheel } from './scroll';
import type { ViewSession } from './session.svelte';

/** The height of a hex row in pixels. */
export const ROW_HEIGHT = 18;

const DEFAULT_ROWS = 32;

/**
 * The bytes of a file around the view, read through `view_bytes`. One read holds the rows in view and a page on each
 * side, so scrolling by rows asks for bytes once per page; a jump asks for the bytes at its target at once.
 */
export class HexModel {
  /** The offset of the row at the top of the view (a multiple of 16). */
  top = $state(0);
  rows = $state(DEFAULT_ROWS);
  /** The bytes held, and the offset of the first one. */
  bytes = $state.raw(new Uint8Array(0));
  base = $state(0);
  ready = $state(false);
  error = $state<string | undefined>();

  readonly #session: ViewSession;
  /** How far the view scrolls at least: the diff of two files lays both out to the longer one's end. */
  readonly #extent: () => number;
  readonly #wheel = new Wheel();
  #generation = 0;

  constructor(session: ViewSession, extent: () => number = () => 0) {
    this.#session = session;
    this.#extent = extent;
  }

  get size(): number {
    return this.#session.size;
  }

  /** The furthest the view scrolls: the offset of the first row of the last page. */
  get limit(): number {
    return lastTop(Math.max(this.size, this.#extent()), this.rows);
  }

  /** Whether the bytes of the rows in view are all held. */
  get #covered(): boolean {
    const want = Math.min(this.size, this.top + this.rows * ROW_BYTES);

    return this.ready && this.base <= this.top && this.base + this.bytes.length >= want;
  }

  /** Puts `offset`'s row at the top (the last page at most) and reads what is missing. */
  moveTo(offset: number): Promise<void> {
    this.top = Math.min(this.limit, rowStart(offset));
    this.#session.position = this.top;

    return this.#fill();
  }

  scrollBy(rows: number): Promise<void> {
    return this.moveTo(this.top + rows * ROW_BYTES);
  }

  /** A wheel turn moves the view by rows. */
  wheel(event: Pick<WheelEvent, 'deltaY' | 'deltaMode'>) {
    const rows = this.#wheel.rows(event, ROW_HEIGHT);
    if (rows !== 0) {
      void this.scrollBy(rows);
    }
  }

  /** The viewing keys; whether the key was one of them. */
  key(event: Parameters<typeof moveOf>[0]): boolean {
    const move = moveOf(event);
    const page = Math.max(1, this.rows - 1);

    if (move === 'lineUp' || move === 'lineDown') {
      void this.scrollBy(move === 'lineUp' ? -1 : 1);
    } else if (move === 'pageUp' || move === 'pageDown') {
      void this.scrollBy(move === 'pageUp' ? -page : page);
    } else if (move === 'home') {
      void this.moveTo(0);
    } else if (move === 'end') {
      void this.moveTo(this.limit);
    }

    return move !== undefined;
  }

  /** Reads the window around the view unless the bytes in view are held. */
  async #fill() {
    // Every move outdates the reads still on their way, covered or not: one asked for a view far from this one
    // would otherwise replace the bytes held with ones that do not cover the rows in view.
    this.#generation += 1;
    const generation = this.#generation;
    if (this.#covered || this.#session.id === undefined) {
      return;
    }

    const offset = Math.max(0, this.top - this.rows * ROW_BYTES);

    try {
      const answer = await readBytes(this.#session, offset, windowBytes(this.rows));
      if (generation !== this.#generation) {
        return;
      }

      this.bytes = new Uint8Array(answer);
      this.base = offset;
      this.ready = true;
      this.error = undefined;
    } catch (error) {
      if (generation === this.#generation) {
        this.error = IpcFailure.from(error).message;
      }
    }
  }
}
