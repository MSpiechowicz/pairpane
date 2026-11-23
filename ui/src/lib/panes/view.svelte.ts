import { CompareMarks } from '$lib/compare/marks.svelte';
import type { Entry } from '$lib/ipc/bindings/Entry';
import type { PaneId } from '$lib/ipc/bindings/PaneId';
import type { Transport } from '$lib/ipc/transport';

import { archiveFile, above } from './archive';
import type { Layout } from './layout';
import { Listing } from './listing.svelte';
import { type MaskMode, maskedFiles } from './mask';
import { type SortColumn, viewOrder } from './order';
import { Selection, summarize } from './selection.svelte';
import { FolderSizes } from './sizes.svelte';
import type { ViewHooks } from './viewHooks';

/** The name the `..` row goes by (no real entry can have it). */
const UP = '..';

/**
 * What a pane shows and does on top of its {@link Listing}: sort, layout, quick filter, cursor, selection, the mask
 * prompt and folder sizes. Rows are numbered as displayed: the `..` row (when there is a parent), then the entries
 * in view order. Selection and sizes are kept by entry name, so they follow the entries through re-sorting and
 * reloads; the entries themselves are never copied or proxied (a folder can hold 100 000).
 */
export class PaneView {
  readonly listing: Listing;
  readonly sizes: FolderSizes;
  /** What a comparison with the other pane marked here; cleared when the pane lists anything. */
  readonly marks = new CompareMarks();

  /** Where the values of the extra columns come from, for sorting by them. */
  readonly #extraOf: ViewHooks['extraOf'];

  #column = $state<SortColumn>('name');
  #desc = $state(false);
  #filter = $state('');

  /** The cursor is the entry named `#name`; `#hint` is where it was last seen, a shortcut and the fallback. */
  #name = $state<string | undefined>();
  #hint = $state(0);
  /** The folder to put the cursor on in the next listing (the one we just left by going up). */
  #cameFrom: string | undefined;
  /** The entry to put the cursor on once a listing has it (a renamed one, which the watch has not listed yet). */
  #pointTo: string | undefined;

  readonly #selected = new Selection();

  /** How the rows are drawn: one per line with its columns, the names alone in several columns, or tiles with thumbnails. A tab brings its own. */
  layout = $state<Layout>('detailed');

  maskMode = $state<MaskMode | undefined>();
  maskText = $state('*');

  /** Indexes into the entries in view order; `null` is the backend order. Recomputed only when its inputs change. */
  readonly #order = $derived.by(() => {
    return viewOrder(this.listing.entries, {
      column: this.#column,
      desc: this.#desc,
      filter: this.#filter,
      sizeOf: (entry) => this.sizes.sizeOf(entry.name),
      extraOf: (entry, column) => this.#extraOf?.(entry, column),
    });
  });

  /** The cursor's row, `-1` when there are no rows. */
  readonly cursor = $derived.by(() => this.#locate());

  /** What the selected entries that are shown add up to (computed folder sizes included); `undefined` when none is. */
  readonly selection = $derived.by(() => {
    const shown = this.selectedShown();
    if (shown.length === 0) {
      return undefined;
    }

    return summarize(shown, (name) => this.sizes.sizeOf(name));
  });

  constructor(ipc: Transport, pane: PaneId, hooks: ViewHooks = {}) {
    this.#extraOf = hooks.extraOf;
    this.sizes = new FolderSizes(ipc, pane, () => this.listing.path);
    this.listing = new Listing(ipc, pane, {
      onOpen: () => {
        this.#cameFrom = undefined; // any open supersedes a going up that has not been listed yet
        hooks.onOpen?.();
      },
      onStart: (reload) => {
        this.marks.clear();
        if (!reload) {
          this.#reset();
        }
        // A reload too: an open of the folder already shown (a tab on the same folder) lists as one. Inside an
        // archive the pane is where its folder is.
        if (this.listing.folder !== undefined) {
          hooks.onListed?.(this.listing.folder);
        }
      },
      onDone: () => {
        this.#prune();
        this.#pointAtPending();
      },
      onFail: hooks.onFail,
      onNotice: hooks.onNotice,
      showHidden: hooks.showHidden,
    });
  }

  // Rows

  /** The `..` row leads to the parent folder (the root of an archive: to its folder); it comes first and is not an entry. */
  get up(): boolean {
    return (this.listing.archive !== undefined || this.listing.parent !== null) && this.listing.status !== 'error';
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
    this.#pointTo = undefined; // the cursor was moved on purpose: a pending pointAt has lost its turn
    this.#name = this.#nameAt(clamped);
    this.#hint = clamped;
  }

  /**
   * Puts the cursor on the entry named `name`. An entry that is not listed yet (just renamed or created) gets it
   * as soon as a listing has it, unless the cursor is moved or another folder opens before.
   */
  pointAt(name: string) {
    const row = this.#find(name);
    if (row >= 0) {
      this.setCursor(row);
    } else {
      this.#pointTo = name;
    }
  }

  #pointAtPending() {
    const name = this.#pointTo;
    if (name !== undefined) {
      this.pointAt(name);
    }
  }

  moveCursor(delta: number) {
    this.setCursor(this.cursor + delta);
  }

  /** Pins the cursor to the row it is on now, after the order changed under it. */
  #keepCursor() {
    this.setCursor(this.cursor);
  }

  // Opening

  /** The archive file in `row`, when it can be opened here (not one inside an archive, nor one among search results). */
  #archiveAt(row: number): string | undefined {
    const { path } = this.listing;
    return path === undefined ? undefined : archiveFile(path, this.entryAt(row));
  }

  /** Whether Enter on `row` opens something: `..`, a folder or an archive file. */
  opens(row: number): boolean {
    return this.isUp(row) || this.entryAt(row)?.kind === 'dir' || this.#archiveAt(row) !== undefined;
  }

  /** Opens the folder in `row` (or the parent for `..`) or the archive; any other file does nothing. */
  open(row: number): Promise<void> {
    if (this.isUp(row)) {
      return this.goUp();
    }

    const entry = this.entryAt(row);
    if (entry?.kind === 'dir') {
      return this.listing.enter(entry.name);
    }

    const archive = this.#archiveAt(row);
    return archive === undefined ? Promise.resolve() : this.listing.openArchive({ archive, dir: '' });
  }

  /**
   * Opens the parent folder, with the cursor on the folder we come from once it is listed. Inside an archive that
   * is the folder above in it; from its root, the folder that holds it, with the cursor on the archive.
   */
  goUp(): Promise<void> {
    const { archive, folder, parent, crumbs } = this.listing;
    if (archive !== undefined && folder !== undefined) {
      const to = above(archive, folder);
      const opened = 'spot' in to ? this.listing.openArchive(to.spot) : this.listing.open(to.folder);
      return this.#leaving(to.left, opened);
    }

    return parent === null ? Promise.resolve() : this.#leaving(crumbs.at(-1)?.name, this.listing.open(parent));
  }

  /** `opened` is an open just made: the cursor goes to `from` in what it lists. */
  #leaving(from: string | undefined, opened: Promise<void>): Promise<void> {
    this.#cameFrom = from; // set after `open`, which forgets the previous one; the first event comes later
    return opened;
  }

  // Selection

  isSelected(name: string): boolean {
    return this.#selected.has(name);
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

  /** The selected entries that pass the filter, in backend order: what the footer counts and a job acts on. */
  selectedShown(): Entry[] {
    const shown: Entry[] = [];
    if (this.#selected.size > 0) {
      this.#eachVisible((entry) => {
        if (this.#selected.has(entry.name)) {
          shown.push(entry);
        }
      });
    }

    return shown;
  }

  /** Deselects the names a job took over (selected entries the quick filter hid stay selected). */
  deselect(names: readonly string[]) {
    this.#selected.remove(names);
  }

  /** Selects exactly the entries named `names` (what a comparison picks): the selection that was there is dropped. */
  select(names: readonly string[]) {
    this.#selected.clear();
    this.#selected.add(names);
  }

  /** Toggles the cursor's entry (never `..`) and returns it. */
  #toggleCursor(): Entry | undefined {
    const entry = this.entryAt(this.cursor);
    if (entry !== undefined) {
      this.#selected.toggle(entry.name);
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
        this.#selected.toggle(entry.name);
      }
    });
  }

  /** Ctrl/Cmd+A: selects every visible entry, folders included. */
  selectAll() {
    const names: string[] = [];
    this.#eachVisible((entry) => names.push(entry.name));
    this.#selected.add(names);
  }

  /** A mouse click on `row`: the cursor, a toggle (Ctrl/Cmd) or the range from the cursor (Shift). */
  pick(row: number, how: 'cursor' | 'toggle' | 'range') {
    if (how === 'range') {
      const from = Math.min(this.cursor, row);
      const to = Math.max(this.cursor, row);
      const names: string[] = [];
      for (let r = from; r <= to; r += 1) {
        const name = this.entryAt(r)?.name;
        if (name !== undefined) {
          names.push(name);
        }
      }

      this.#selected.add(names);
    }

    this.setCursor(row);

    if (how === 'toggle') {
      const name = this.entryAt(row)?.name;
      if (name !== undefined) {
        this.#selected.toggle(name);
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

    const names = maskedFiles((visit) => this.#eachVisible(visit), this.maskText);

    if (mode === 'select') {
      this.#selected.add(names);
    } else {
      this.#selected.remove(names);
    }

    this.maskMode = undefined;
  }

  // Listing hooks

  /** Another folder: selection, filter, sizes and the mask prompt belong to the old one. */
  #reset() {
    this.#selected.clear();
    this.#filter = '';
    this.maskMode = undefined;

    this.sizes.clear();

    this.#pointTo = undefined;
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

    this.#selected.retain(present);
    this.sizes.prune(present);
  }
}
