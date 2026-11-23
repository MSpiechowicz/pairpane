import type { Transport } from '$lib/ipc/transport';

/**
 * What makes a window close worth asking about: the editor or the diff (either of its two files) holds text that is
 * not saved, or transfers are running and the user wants to be asked before they are stopped.
 */
export type Owner = 'editor' | 'diff' | 'transfers';

/**
 * Tells the backend whether a window close would lose something, so that it asks before it closes. Several owners
 * can hold something at once; the backend hears only of the one fact (any owner dirty) and only when it changes.
 */
export class Unsaved {
  readonly #ipc: Transport;
  readonly #owners: Record<Owner, boolean> = { editor: false, diff: false, transfers: false };
  #told = false;

  constructor(ipc: Transport) {
    this.#ipc = ipc;
  }

  /** Whether any owner holds changes. */
  get dirty(): boolean {
    return this.#owners.editor || this.#owners.diff || this.#owners.transfers;
  }

  /** An owner's changes appeared or went. */
  report(owner: Owner, dirty: boolean) {
    this.#owners[owner] = dirty;
    if (this.dirty !== this.#told) {
      this.#told = this.dirty;
      this.#send(this.dirty);
    }
  }

  /**
   * A close that was asked about is given up with changes still there: telling the backend again that there are
   * changes ends its pending close, so a later close asks again (an unanswered one is given up after a while and
   * would let the window close without asking).
   */
  stillDirty() {
    if (this.dirty) {
      this.#send(true);
    }
  }

  #send(dirty: boolean) {
    this.#ipc.invoke('editor_set_dirty', { dirty }).catch(() => {});
  }
}
