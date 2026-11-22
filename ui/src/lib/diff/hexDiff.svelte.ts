import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { retryBusy } from '$lib/viewer/busy';
import { offsetDigits, ROW_BYTES, rowStart } from '$lib/viewer/hex';
import { HexModel } from '$lib/viewer/hexWindow.svelte';
import { ViewSession } from '$lib/viewer/session.svelte';
import type { Target } from '$lib/viewer/viewer.svelte';

import { type DiffRow, diffRow } from './hexDiff';

/** What the diff says when its sessions are gone from the backend (it closed them, or the app did). */
export const CLOSED_NOTICE = 'The comparison was closed; open it again.';

/** The model of the left file: moving it moves the right file's too, so the two columns always show the same rows. */
class Leading extends HexModel {
  follower: HexModel | undefined;

  override async moveTo(offset: number): Promise<void> {
    await Promise.all([super.moveTo(offset), this.follower?.moveTo(offset)]);
  }
}

/**
 * The hex diff of two files: a view session on each, the two read through `view_bytes` around one shared top, and
 * the search for the next and previous differing byte, which the backend does in steps (`searching` asks again
 * from where it stopped, so that a multi-gigabyte comparison never holds a window call for long).
 */
export class HexDiff {
  readonly left: ViewSession;
  readonly right: ViewSession;
  /** The model that is moved: keys and the wheel go to it. */
  readonly lead: Leading;
  readonly follower: HexModel;

  /** The difference that Previous/Next went to last. */
  current = $state<number | undefined>();
  /** Where the first difference is (`none`: there is none), `undefined` while that is being looked for. */
  first = $state<number | 'none' | undefined>();
  /** How far a long search for the next difference has got (it has not answered within one step), while one runs. */
  searching = $state<number | undefined>();
  /** What the last move said: that there is no further difference, or why it failed. */
  message = $state<string | undefined>();

  readonly #ipc: Transport;
  #ticket = 0;
  #closed = false;
  #begun = false;

  constructor(ipc: Transport, left: Target, right: Target) {
    this.#ipc = ipc;
    this.left = new ViewSession(ipc, left.path, left.name);
    this.right = new ViewSession(ipc, right.path, right.name);

    const extent = () => this.size;
    this.lead = new Leading(this.left, extent);
    this.follower = new HexModel(this.right, extent);
    this.lead.follower = this.follower;
  }

  get status(): 'opening' | 'ready' | 'failed' {
    if (this.left.status === 'failed' || this.right.status === 'failed') {
      return 'failed';
    }

    return this.left.status === 'ready' && this.right.status === 'ready' ? 'ready' : 'opening';
  }

  /** Why a file could not be opened. */
  get failure(): string | undefined {
    return this.left.error ?? this.right.error;
  }

  /** The length the columns are laid out to: the longer file's. */
  get size(): number {
    return Math.max(this.left.size, this.right.size);
  }

  get top(): number {
    return this.lead.top;
  }

  /** How many rows fit; both columns read for the same ones. */
  setRows(rows: number) {
    this.lead.rows = rows;
    this.follower.rows = rows;
  }

  /** The rows in view whose bytes both files hold (a row past a file's end needs none of it). */
  readonly lines: DiffRow[] = $derived.by(() => {
    const digits = offsetDigits(this.size);
    const covers = (model: HexModel, offset: number) =>
      offset >= model.size || (model.ready && offset >= model.base && offset < model.base + model.bytes.length);
    const rows: DiffRow[] = [];

    for (let index = 0; index <= this.lead.rows; index += 1) {
      const offset = this.lead.top + index * ROW_BYTES;
      if (offset < this.size && covers(this.lead, offset) && covers(this.follower, offset)) {
        rows.push(
          diffRow(offset, { bytes: this.lead.bytes, base: this.lead.base }, { bytes: this.follower.bytes, base: this.follower.base }, digits, this.current),
        );
      }
    }

    return rows;
  });

  /** Looks for the first difference (once, when both files are open). */
  begin() {
    if (this.#begun || this.status !== 'ready') {
      return;
    }

    this.#begun = true;
    void this.#first();
  }

  async #first() {
    try {
      const found = await this.#search(true, 0, () => !this.#closed);
      if (found !== 'stale') {
        this.first = found;
      }
    } catch (error) {
      // The size line goes on saying that it is looking; the message says why it will not find out.
      if (!this.#closed) {
        this.message = this.#said(error);
      }
    }
  }

  /** Previous difference: the last one before the one gone to last (or before the top of the view). */
  previous() {
    void this.#step(false);
  }

  /** Next difference: the first one after the one gone to last (or from the top of the view). */
  next() {
    void this.#step(true);
  }

  async #step(forward: boolean) {
    this.#ticket += 1;
    const ticket = this.#ticket;
    const alive = () => ticket === this.#ticket && !this.#closed;
    const from = this.current === undefined ? this.top : this.current + (forward ? 1 : 0);

    this.message = undefined;

    try {
      const found = await this.#search(forward, from, alive, (at) => (this.searching = at));
      if (found === 'stale') {
        return;
      }

      if (found === 'none') {
        this.message = forward ? 'No further difference.' : 'No earlier difference.';
      } else {
        this.current = found;
        const lead = Math.floor(this.lead.rows / 3) * ROW_BYTES;
        void this.lead.moveTo(Math.max(0, rowStart(found) - lead));
      }
    } catch (error) {
      if (alive()) {
        this.message = this.#said(error);
      }
    } finally {
      if (alive()) {
        this.searching = undefined;
      }
    }
  }

  /**
   * Asks for the next (previous) differing byte from `from`, and again from where the backend stopped, until it
   * answers; `stale` when `alive` says nobody wants the answer any more.
   */
  async #search(forward: boolean, from: number, alive: () => boolean, onProgress?: (at: number) => void): Promise<number | 'none' | 'stale'> {
    let at = from;

    while (alive()) {
      const [left, right] = [this.left.id, this.right.id];
      if (left === undefined || right === undefined) {
        return 'stale';
      }

      const answer = await retryBusy(() => this.#ipc.invoke('diff_next_difference', { left, right, from: at, forward }));
      if (!alive()) {
        break;
      }

      if (answer.kind === 'searching') {
        at = answer.from;
        onProgress?.(at);
      } else {
        return answer.kind === 'found' ? answer.offset : 'none';
      }
    }

    return 'stale';
  }

  #said(error: unknown): string {
    const failure = IpcFailure.from(error);

    return failure.code === 'notFound' ? CLOSED_NOTICE : failure.message;
  }

  /** Ends the sessions on the backend, and every search that still runs. */
  close() {
    this.#closed = true;
    this.#ticket += 1;
    this.left.close();
    this.right.close();
  }
}
