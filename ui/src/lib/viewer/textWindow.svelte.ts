import type { TextLine } from '$lib/ipc/bindings/TextLine';
import type { TextWindow } from '$lib/ipc/bindings/TextWindow';
import { IpcFailure } from '$lib/ipc/transport';

import { retryBusy } from './busy';
import { moveOf, Wheel } from './scroll';
import type { ViewSession } from './session.svelte';

/** The height of a text row in pixels; the view scrolls by whole lines. */
export const LINE_HEIGHT = 18;

/** The rows assumed before the view has been measured. */
const DEFAULT_ROWS = 40;
const MIN_FETCH = 100;
const MAX_FETCH = 1000;
/** The lines kept around the view, in windows: more is dropped from the side the view moves away from. */
const KEEP_WINDOWS = 4;

/**
 * How many lines to drop (`cut`, at most) so that the cut falls between lines: `inside(at)` says whether a cut of `at`
 * lines would fall inside a long line. A cut is moved back to a line start when one is within reach; when none is (a
 * file of one huge line, every piece of it continuing), it falls between two pieces, so the buffer stays bounded. The
 * backend continues a window from a piece's offset, so that is no loss.
 */
function wholeLines(cut: number, inside: (at: number) => boolean): number {
  let at = cut;
  while (at > 0 && inside(at)) {
    at -= 1;
  }

  return at > 0 ? at : cut;
}

/**
 * The lines of a text file around the view, fetched from the backend a window at a time. The model keeps a buffer of
 * lines that grows towards where the view moves (forward from the buffer's `end`, backward from its start, without
 * snapping) and drops what is far behind, so a scroll costs a request once in a few pages, never one per line. A jump
 * (the scroll bar, Home, End, a search hit, a change of encoding) replaces the buffer; the answer of a request that
 * a jump has outdated is dropped.
 */
export class TextModel {
  lines = $state.raw<TextLine[]>([]);
  /** The index in `lines` of the row at the top of the view. */
  first = $state(0);
  atStart = $state(true);
  atEnd = $state(true);
  /** The first window has arrived (an empty file has no lines but is ready). */
  ready = $state(false);
  error = $state<string | undefined>();
  rows = $state(DEFAULT_ROWS);

  readonly #session: ViewSession;
  /** The byte offsets the buffer starts at and ends before: where the next window is asked from. */
  #start = 0;
  #end = 0;
  /** Bumped by every jump; a window asked before it is stale. */
  #generation = 0;
  /** A jump has been asked and its window has not come: the buffer is about to be replaced, so it is not extended. */
  #jumping = false;
  #wanting = false;
  readonly #wheel = new Wheel();

  constructor(session: ViewSession) {
    this.#session = session;
  }

  get #want(): number {
    return Math.min(MAX_FETCH, Math.max(MIN_FETCH, this.rows * 3));
  }

  /** The furthest `first` goes: the last row at the bottom, unless wrapping may make a row taller than it looks. */
  get #limit(): number {
    const count = this.lines.length;
    if (count === 0) {
      return 0;
    }

    return this.atEnd && !this.#session.wrap ? Math.max(0, count - this.rows) : count - 1;
  }

  /** The offset of the line at the top of the view. */
  get top(): number {
    return this.lines[this.first]?.offset ?? this.#start;
  }

  /** Where the scroll bar's thumb is: the end of the file when the view is at the bottom, else the top line. */
  get thumb(): number {
    const bottom = this.atEnd && this.first >= this.#limit;
    const alone = this.atStart && this.first === 0;

    return bottom && !alone ? this.#session.size : this.top;
  }

  async #ask(offset: number, backward: boolean, snap: boolean): Promise<TextWindow | undefined> {
    const session = this.#session;
    const id = session.id;
    if (id === undefined) {
      return undefined;
    }

    try {
      const query = { encoding: session.encoding, offset, backward, lines: this.#want, snap };
      const window = await retryBusy(() => session.ipc.invoke('view_text', { session: id, query }));
      this.error = undefined;

      return window;
    } catch (error) {
      this.error = IpcFailure.from(error).message;

      return undefined;
    }
  }

  /** Replaces the buffer by the window at `offset` (a forward one), or the one that ends there (a backward one). */
  async #jump(offset: number, backward: boolean, snap: boolean) {
    this.#generation += 1;
    const generation = this.#generation;
    this.#jumping = true;

    const window = await this.#ask(offset, backward, snap);
    if (generation !== this.#generation) {
      return; // a newer jump is pending and owns the flag
    }

    this.#jumping = false;
    if (window === undefined) {
      return;
    }

    this.lines = window.lines;
    this.#start = window.start;
    this.#end = window.end;
    this.atStart = window.atStart;
    this.atEnd = window.atEnd;
    this.first = backward ? Math.max(0, window.lines.length - this.rows) : 0;
    this.ready = true;
    this.#sync();
  }

  /** The view at a scroll bar's offset: the line it falls in; the end of the scroll bar is the end of the file. */
  seek(offset: number): Promise<void> {
    return offset >= this.#session.size ? this.end() : this.#jump(offset, false, true);
  }

  /** The view at the byte offset (a search hit, the other view's position, the same place in another encoding). */
  restart(offset: number): Promise<void> {
    return this.#jump(offset, false, true);
  }

  home(): Promise<void> {
    return this.#jump(0, false, false);
  }

  end(): Promise<void> {
    return this.#jump(this.#session.size, true, false);
  }

  /** Moves the view by `delta` lines, asking for more of the file when it comes near the edge of the buffer. */
  scrollBy(delta: number) {
    this.first = Math.min(this.#limit, Math.max(0, this.first + delta));
    this.#sync();
    void this.#refill();
  }

  /** A wheel turn moves the view by lines. */
  wheel(event: Pick<WheelEvent, 'deltaY' | 'deltaMode'>) {
    const lines = this.#wheel.rows(event, LINE_HEIGHT);
    if (lines !== 0) {
      this.scrollBy(lines);
    }
  }

  /** The viewing keys; whether the key was one of them. */
  key(event: Parameters<typeof moveOf>[0]): boolean {
    const move = moveOf(event);
    const page = Math.max(1, this.rows - 1);

    if (move === 'lineUp' || move === 'lineDown') {
      this.scrollBy(move === 'lineUp' ? -1 : 1);
    } else if (move === 'pageUp' || move === 'pageDown') {
      this.scrollBy(move === 'pageUp' ? -page : page);
    } else if (move === 'home') {
      void this.home();
    } else if (move === 'end') {
      void this.end();
    }

    return move !== undefined;
  }

  #sync() {
    this.#session.position = this.top;
  }

  async #refill() {
    if (this.#wanting || this.#jumping || !this.ready) {
      return;
    }

    const after = this.lines.length - (this.first + this.rows);
    if (!this.atEnd && after < this.rows) {
      await this.#extend(false);
    } else if (!this.atStart && this.first < this.rows) {
      await this.#extend(true);
    }
  }

  /** Adds the next window at the end of the buffer (or the one before its start), dropping the far side. */
  async #extend(backward: boolean) {
    this.#wanting = true;
    const generation = this.#generation;
    const window = await this.#ask(backward ? this.#start : this.#end, backward, false);
    this.#wanting = false;
    if (window === undefined) {
      return;
    }

    if (generation !== this.#generation) {
      // A jump replaced the buffer meanwhile; whatever scrolling it left to do was held back by this request.
      await this.#refill();

      return;
    }

    if (backward) {
      this.#prepend(window);
    } else {
      this.#append(window);
    }
    this.#sync();

    // The view may have moved on while the window was on its way.
    if (window.lines.length > 0) {
      await this.#refill();
    }
  }

  #append(window: TextWindow) {
    const lines = [...this.lines, ...window.lines];
    this.#end = window.end;
    this.atEnd = window.atEnd;

    const room = Math.min(lines.length - this.#want * KEEP_WINDOWS, this.first);
    const cut = wholeLines(room, (at) => lines[at - 1].continues);

    if (cut > 0) {
      this.lines = lines.slice(cut);
      this.first -= cut;
      this.atStart = false;
      this.#start = lines[cut].offset;
    } else {
      this.lines = lines;
    }
  }

  #prepend(window: TextWindow) {
    const lines = [...window.lines, ...this.lines];
    this.first += window.lines.length;
    this.#start = window.start;
    this.atStart = window.atStart;

    const room = Math.min(lines.length - this.#want * KEEP_WINDOWS, lines.length - (this.first + this.rows));
    const cut = wholeLines(room, (at) => lines[lines.length - at - 1].continues);

    if (cut > 0) {
      this.#end = lines[lines.length - cut].offset;
      this.atEnd = false;
      this.lines = lines.slice(0, lines.length - cut);
    } else {
      this.lines = lines;
    }
  }
}
