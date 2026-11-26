import { tick } from 'svelte';

import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { focusActiveList } from './focus';
import type { PaneView } from './view.svelte';

/** The row being renamed: the pane and tab it is in, the folder and the name it had when F2 was pressed. */
export interface Editing {
  view: PaneView;
  tab: number;
  path: string;
  name: string;
  dir: boolean;
}

/**
 * Renaming an entry in its row. The entry is found by name (the rows of a pane are keyed by index, so the field
 * may move to another row when the order changes). Enter renames, Escape or leaving the field cancels, and a
 * rename that is refused leaves the field open and says why through `notify`. Only one field is open at a time.
 */
export class InlineRename {
  current = $state.raw<Editing | undefined>();
  value = $state('');
  /** The backend is renaming: the field cannot be left or changed. */
  busy = $state(false);

  readonly #ipc: Transport;
  readonly #notify: (text: string) => void;
  /** The field that is on screen, if any. */
  #field: HTMLElement | undefined;

  constructor(ipc: Transport, notify: (text: string) => void) {
    this.#ipc = ipc;
    this.#notify = notify;
  }

  /** Opens the field on the entry under `view`'s cursor; `..` and an empty row have none. */
  begin(view: PaneView, tab: number) {
    const under = view.entryAt(view.cursor);
    const { path, remote } = view.listing;
    if (this.busy || under === undefined || path === undefined || remote !== undefined) {
      return;
    }

    this.current = { view, tab, path, name: under.name, dir: under.kind === 'dir' };
    this.value = under.name;
  }

  /** Closes the field without renaming; it cannot be called off while the backend is at it. */
  cancel() {
    if (!this.busy) {
      this.current = undefined;
    }
  }

  async commit() {
    const edit = this.current;
    const to = this.value;
    if (edit === undefined || this.busy) {
      return;
    }

    if (to === edit.name) {
      this.cancel();
      return;
    }

    if (to.trim() === '') {
      return;
    }

    this.busy = true;
    try {
      await this.#ipc.invoke('ops_rename', { path: edit.path, from: edit.name, to });
      this.current = undefined;
      edit.view.pointAt(to);
    } catch (error) {
      this.#notify(IpcFailure.from(error).message);
    } finally {
      this.busy = false;
    }
  }

  /**
   * Takes the focus for the field; hands it back to the list when the field is gone, and cancels when it is gone
   * for good (not just moved to another row of the pane). The result is the field's teardown.
   */
  mount(input: HTMLInputElement): () => void {
    this.#field = input;
    input.focus();

    return () => {
      if (this.#field === input) {
        this.#field = undefined;
      }

      void tick().then(() => {
        if (this.#field === undefined) {
          this.cancel();
          focusActiveList();
        }
      });
    };
  }
}
