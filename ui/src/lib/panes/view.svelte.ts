import type { Entry } from '$lib/ipc/bindings/Entry';
import type { PaneId } from '$lib/ipc/bindings/PaneId';
import type { Transport } from '$lib/ipc/transport';

import { Listing } from './listing.svelte';
import { parseMask } from './mask';
import { type SortColumn, viewOrder } from './order';
import { FolderSizes } from './sizes.svelte';

/** The name the `..` row goes by (no real entry can have it). */
const UP = '..';

export type MaskMode = 'select' | 'deselect';

/** What the owner of a pane (its tabs) needs to hear about its folders. */
export interface ViewHooks {
  /** An open begins (any open, a silent one included), before any of its events. */
  onOpen?: () => void;
  /** A folder started listing, the shown one again included: where the pane is now. */
  onListed?: (path: string) => void;
  /** An open failed and the pane has nothing to show. */
  onFail?: () => void;
  /** An open of another folder failed and the pane stays in the folder it had. */
  onNotice?: () => void;
}

/**
 * What a pane shows and does on top of its {@link Listing}: sort, quick filter, cursor, selection, the mask
 * prompt and folder sizes. Rows are numbered as displayed: the `..` row (when there is a parent), then the entries
 * in view order. Selection and sizes are kept by entry name, so they follow the entries through re-sorting and
 * reloads; the entries themselves are never copied or proxied (a folder can hold 100 000).
 */
export class PaneView {
  readonly listing: Listing;
  readonly sizes: FolderSizes;

  #column = $state<SortColumn>('name');
  #desc = $state(false);
  #filter = $state('');

  /** The cursor is the entry named `#name`; `#hint` is where it was last seen, a shortcut and the fallback. */
  #name = $state<string | undefined>();
  #hint = $state(0);
  /** The folder to put the cursor on in the next listing (the one we just left by going up). */
  #cameFrom: string | undefined;

  /** Plain set (not proxied); readers track `#selection`, bumped on every change. */
  readonly #selected = new Set<string>();
  #selection = $state(0);

  maskMode = $state<MaskMode | undefined>();
  maskText = $state('*');

  /** Indexes into the entries in view order; `null` is the backend order. Recomputed only when its inputs change. */
  readonly #order = $derived.by(() => {
    return viewOrder(this.listing.entries, {
      column: this.#column,
      desc: this.#desc,
      filter: this.#filter,
      sizeOf: (entry) => this.sizes.sizeOf(entry.name),
    });
  });

  /** The cursor's row, `-1` when there are no rows. */
  readonly cursor = $derived.by(() => this.#locate());

  /** What the selected entries add up to (computed folder sizes included); `undefined` when none is selected. */
  readonly selection = $derived.by(() => {
    void this.#selection;
    if (this.#selected.size === 0) {
      return undefined;
    }

    let dirs = 0;
    let files = 0;
    let bytes = 0;
    for (const entry of this.listing.entries) {
      if (entry === undefined || !this.#selected.has(entry.name)) {
        continue;
      }

      if (entry.kind === 'dir') {
        dirs += 1;
        bytes += this.sizes.sizeOf(entry.name) ?? 0;
      } else {
        files += 1;
        bytes += entry.size;
      }
    }

    return { dirs, files, bytes };
  });

  constructor(ipc: Transport, pane: PaneId, hooks: ViewHooks = {}) {
    this.sizes = new FolderSizes(ipc, pane, () => this.listing.path);
    this.listing = new Listing(ipc, pane, {
      onOpen: () => {
        this.#cameFrom = undefined; // any open supersedes a going up that has not been listed yet
        hooks.onOpen?.();
      },
      onStart: (reload) => {
        if (!reload) {
          this.#reset();
        }
        // A reload too: an open of the folder already shown (a tab on the same folder) lists as one.
        if (this.listing.path !== undefined) {
          hooks.onListed?.(this.listing.path);
        }
      },
      onDone: () => this.#prune(),
      onFail: hooks.onFail,
      onNotice: hooks.onNotice,
    });
  }

  // Rows

  /** The `..` row leads to the parent folder; it comes first and is not an entry. */
  get up(): boolean {
    return this.listing.parent !== null && this.listing.status !== 'error';
  }

  get count(): number {
    return (this.up ? 1 : 0) + (this.#order ? this.#order.length : this.listing.entries.length);
  }

  /** The entries that pass the filter. */
  get matches(): number {
    return this.#order ? this.#order.length : this.listing.entries.length;
  }

  isUp(row: number): boolean {
    return row === 0 && this.up;
  }

  /** The entry shown in `row`; `undefined` for `..` and for an entry whose batch has not arrived yet. */
  entryAt(row: number): Entry | undefined {
    const index = row - (this.up ? 1 : 0);
    if (index < 0) {
      return undefined;
    }

    const slot = this.#order ? this.#order[index] : index;
    return slot === undefined ? undefined : this.listing.entries[slot];
  }

  #nameAt(row: number): string | undefined {
    return this.isUp(row) ? UP : this.entryAt(row)?.name;
  }

  // Sort and filter

  get column(): SortColumn {
    return this.#column;
  }

  get desc(): boolean {
    return this.#desc;
  }

  /** The same column flips the direction, another one starts ascending. */
  sortBy(column: SortColumn) {
    if (column === this.#column) {
      this.#desc = !this.#desc;
    } else {
      this.#column = column;
      this.#desc = false;
    }

    this.#keepCursor();
  }

  /** Sets the sort without toggling (a tab brings its own); the cursor stays on its entry. */
  restoreSort(column: SortColumn, desc: boolean) {
    this.#column = column;
    this.#desc = desc;
    this.#keepCursor();
  }

  get filter(): string {
    return this.#filter;
  }

  /** A filter with matches puts the cursor on the first one when it was on `..` (an entry it hides loses it too). */
  setFilter(text: string) {
    this.#filter = text;

    if (text !== '' && this.matches > 0 && this.isUp(this.cursor)) {
      this.setCursor(1);
    } else {
      this.#keepCursor();
    }
  }

  // Cursor

  /** The cursor's entry is found by name; when the order changed it is searched, otherwise it is where it was. */
  #locate(): number {
    const count = this.count;
    if (count === 0) {
      return -1;
    }

    const name = this.#name;
    if (name !== undefined) {
      if (this.#nameAt(this.#hint) === name) {
        return this.#hint;
      }

      const found = this.#find(name);
      if (found >= 0) {
        return found;
      }

      // Filtered out: the first match takes over.
      if (this.#filter !== '') {
        return Math.min(this.up ? 1 : 0, count - 1);
      }
    }

    return Math.min(Math.max(this.#hint, 0), count - 1);
  }

  #find(name: string): number {
    if (name === UP) {
      return this.up ? 0 : -1;
    }

    for (let row = this.up ? 1 : 0; row < this.count; row += 1) {
      if (this.#nameAt(row) === name) {
        return row;
      }
    }

    return -1;
  }

  setCursor(row: number) {
    const count = this.count;
    if (count === 0) {
      return;
    }

    const clamped = Math.min(Math.max(row, 0), count - 1);
    this.#name = this.#nameAt(clamped);
    this.#hint = clamped;
  }

  moveCursor(delta: number) {
    this.setCursor(this.cursor + delta);
  }

  /** Pins the cursor to the row it is on now, after the order changed under it. */
  #keepCursor() {
    this.setCursor(this.cursor);
  }

  // Opening

  /** Opens the folder in `row` (or the parent for `..`); a file does nothing. */
  open(row: number): Promise<void> {
    if (this.isUp(row)) {
      return this.goUp();
    }

    const entry = this.entryAt(row);
    return entry?.kind === 'dir' ? this.listing.enter(entry.name) : Promise.resolve();
  }

  /** Opens the parent folder, with the cursor on the folder we come from once it is listed. */
  goUp(): Promise<void> {
    const parent = this.listing.parent;
    if (parent === null) {
      return Promise.resolve();
    }

    const from = this.listing.crumbs.at(-1)?.name;
    const opened = this.listing.open(parent);
    this.#cameFrom = from; // set after `open`, which forgets the previous one; the first event comes later
    return opened;
  }

  // Selection

  isSelected(name: string): boolean {
    void this.#selection;
    return this.#selected.has(name);
  }

  #toggle(name: string) {
    if (!this.#selected.delete(name)) {
      this.#selected.add(name);
    }

    this.#selection += 1;
  }

  /** Visits the visible entries (those passing the filter) in backend order. */
  #eachVisible(visit: (entry: Entry) => void) {
    const entries = this.listing.entries;
    const order = this.#order;
    const length = order ? order.length : entries.length;

    for (let i = 0; i < length; i += 1) {
      const entry = entries[order ? order[i] : i];
      if (entry !== undefined) {
        visit(entry);
      }
    }
  }

  /** Toggles the cursor's entry (never `..`) and returns it. */
  #toggleCursor(): Entry | undefined {
    const entry = this.entryAt(this.cursor);
    if (entry !== undefined) {
      this.#toggle(entry.name);
    }

    return entry;
  }

  /** Space: toggles the cursor's entry and moves down; a folder is also measured. */
  space() {
    const entry = this.#toggleCursor();
    if (entry?.kind === 'dir') {
      this.sizes.measure(entry.name);
    }

    this.moveCursor(1);
  }

  /** Insert: toggles the cursor's entry and moves down. */
  insert() {
    this.#toggleCursor();
    this.moveCursor(1);
  }

  /** `*`: inverts the selection of the visible files. */
  invert() {
    this.#eachVisible((entry) => {
      if (entry.kind !== 'dir') {
        this.#toggle(entry.name);
      }
    });
  }

  /** Ctrl/Cmd+A: selects every visible entry, folders included. */
  selectAll() {
    this.#eachVisible((entry) => this.#selected.add(entry.name));
    this.#selection += 1;
  }

  /** A mouse click on `row`: the cursor, a toggle (Ctrl/Cmd) or the range from the cursor (Shift). */
  pick(row: number, how: 'cursor' | 'toggle' | 'range') {
    if (how === 'range') {
      const from = Math.min(this.cursor, row);
      const to = Math.max(this.cursor, row);
      for (let r = from; r <= to; r += 1) {
        const name = this.entryAt(r)?.name;
        if (name !== undefined) {
          this.#selected.add(name);
        }
      }

      this.#selection += 1;
    }

    this.setCursor(row);

    if (how === 'toggle') {
      const name = this.entryAt(row)?.name;
      if (name !== undefined) {
        this.#toggle(name);
      }
    }
  }

  // Mask

  beginMask(mode: MaskMode) {
    this.maskMode = mode;
    this.maskText = '*';
  }

  cancelMask() {
    this.maskMode = undefined;
  }

  /** Selects or deselects the visible files matching the mask. */
  applyMask() {
    const mode = this.maskMode;
    if (mode === undefined) {
      return;
    }

    const matches = parseMask(this.maskText);
    this.#eachVisible((entry) => {
      if (entry.kind === 'dir' || !matches(entry.name)) {
        return;
      }

      if (mode === 'select') {
        this.#selected.add(entry.name);
      } else {
        this.#selected.delete(entry.name);
      }
    });

    this.#selection += 1;
    this.maskMode = undefined;
  }

  // Listing hooks

  /** Another folder: selection, filter, sizes and the mask prompt belong to the old one. */
  #reset() {
    this.#selected.clear();
    this.#selection += 1;
    this.#filter = '';
    this.maskMode = undefined;

    this.sizes.clear();

    this.#name = this.#cameFrom;
    this.#cameFrom = undefined;
    this.#hint = 0;
  }

  /** A finished listing: names that are no longer there stop being selected and lose their folder size. */
  #prune() {
    if (this.#selected.size === 0 && !this.sizes.any) {
      return;
    }

    const present = new Set<string>();
    for (const entry of this.listing.entries) {
      if (entry !== undefined) {
        present.add(entry.name);
      }
    }

    for (const name of [...this.#selected]) {
      if (!present.has(name)) {
        this.#selected.delete(name);
      }
    }

    this.#selection += 1;
    this.sizes.prune(present);
  }
}
