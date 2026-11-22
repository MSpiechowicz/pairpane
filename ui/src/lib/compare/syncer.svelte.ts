import type { CompareEvent } from '$lib/ipc/bindings/CompareEvent';
import type { CompareId } from '$lib/ipc/bindings/CompareId';
import type { CompareRoot } from '$lib/ipc/bindings/CompareRoot';
import type { CompareStatus } from '$lib/ipc/bindings/CompareStatus';
import type { SyncItem } from '$lib/ipc/bindings/SyncItem';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { parseMask } from '$lib/panes/mask';

import {
  type Archives,
  baseName,
  defaultAction,
  itemsOf,
  type SyncAction,
  type SyncRow,
  totalsOf,
  type Totals,
  whyNot,
} from './plan';

export type SyncStatus = 'idle' | 'running' | 'done' | 'failed';

/** How a scan ended. */
export interface SyncSummary {
  rows: number;
  skipped: number;
  truncated: boolean;
  cancelled: boolean;
}

/** The two folders compared. */
export interface Roots {
  left: CompareRoot;
  right: CompareRoot;
}

const STATUSES: readonly CompareStatus[] = ['leftOnly', 'rightOnly', 'leftNewer', 'rightNewer', 'differs', 'equal'];

const noCounts = () => Object.fromEntries(STATUSES.map((status) => [status, 0])) as Record<CompareStatus, number>;

/**
 * The Synchronize dialog's state: the folders and the options of the scan, the scan that runs and its rows as they
 * stream in, what is to be done about each row, and the copy job that is asked for in the end. At most one scan is
 * the dialog's: starting another, or closing the dialog, cancels it and drops it from the backend.
 */
export class Syncer {
  /** The folders compared; `undefined` while the dialog is closed. */
  roots = $state.raw<Roots | undefined>();
  subfolders = $state(true);
  content = $state(false);
  hidden = $state(false);
  /** Masks of the names (`*.txt; *.md`) that narrow the rows shown; the scan does not know of it. */
  nameFilter = $state('');
  /** The statuses shown; what is equal is not, to begin with. */
  readonly shown = $state<Record<CompareStatus, boolean>>({
    leftOnly: true,
    rightOnly: true,
    leftNewer: true,
    rightNewer: true,
    differs: true,
    equal: false,
  });
  status = $state<SyncStatus>('idle');
  /** Asked to stop; the scan ends with its `done` shortly. */
  stopping = $state(false);
  progress = $state<{ folders: number; files: number; bytes: number } | undefined>();
  summary = $state<SyncSummary | undefined>();
  failure = $state<string | undefined>();
  /** What the last bulk change of actions could not do. */
  notice = $state<string | undefined>();
  /** The copies wait for a yes. */
  confirming = $state(false);

  readonly #ipc: Transport;
  /** A plain array filled in place (100 000 rows must not be proxied); readers track `#revision`. */
  #rows: SyncRow[] = [];
  #revision = $state(0);
  #counts = noCounts();
  #archives: Archives = { left: false, right: false };
  #id: CompareId | undefined;
  /** Tells the events of the latest scan from those of one that was dropped. */
  #run = 0;

  /** The indexes of the rows that pass the filters, in the order of the scan. */
  readonly #view = $derived.by(() => {
    void this.#revision;
    const shown = $state.snapshot(this.shown);
    const matches = this.nameFilter.trim() === '' ? undefined : parseMask(this.nameFilter);
    const indexes: number[] = [];

    this.#rows.forEach(({ row }, index) => {
      if (shown[row.status] && (matches === undefined || matches(baseName(row.path)))) {
        indexes.push(index);
      }
    });

    return indexes;
  });

  readonly #totals = $derived.by(() => {
    void this.#revision;
    return totalsOf(this.#rows);
  });

  constructor(ipc: Transport) {
    this.#ipc = ipc;
  }

  /** The dialog is open and has the keyboard. */
  get modal(): boolean {
    return this.roots !== undefined;
  }

  get running(): boolean {
    return this.status === 'running';
  }

  /** A side is an archive: its files cannot be compared by content. */
  get contentAllowed(): boolean {
    return !this.#archives.left && !this.#archives.right;
  }

  get archives(): Archives {
    return this.#archives;
  }

  /** The rows that pass the filters. */
  get count(): number {
    return this.#view.length;
  }

  /** How many rows the scan delivered. */
  get total(): number {
    void this.#revision;
    return this.#rows.length;
  }

  /** The row shown at `index`. */
  at(index: number): SyncRow | undefined {
    void this.#revision;
    const slot = this.#view[index];
    return slot === undefined ? undefined : this.#rows[slot];
  }

  /** How many rows each status has (the filters leave them be). */
  get counts(): Readonly<Record<CompareStatus, number>> {
    void this.#revision;
    return this.#counts;
  }

  get totals(): Totals {
    return this.#totals;
  }

  /** The copies that are asked for. */
  get items(): SyncItem[] {
    void this.#revision;
    return itemsOf(this.#rows);
  }

  /** What is to be done about `entry` (reactive: the rows are plain objects). */
  actionOf(entry: SyncRow): SyncAction {
    void this.#revision;
    return entry.action;
  }

  /** Whether `entry` is picked (reactive). */
  pickedOf(entry: SyncRow): boolean {
    void this.#revision;
    return entry.picked;
  }

  /** How many of the rows shown are picked. */
  get picked(): number {
    void this.#revision;
    return this.#view.reduce((sum, slot) => sum + (this.#rows[slot].picked ? 1 : 0), 0);
  }

  /** Opens the dialog on `roots`, with no rows. */
  show(roots: Roots, hidden: boolean) {
    if (this.roots === undefined) {
      this.#clear();
      this.roots = roots;
      this.hidden = hidden;
      this.#archives = { left: roots.left.kind === 'archive', right: roots.right.kind === 'archive' };
      this.content = this.content && this.contentAllowed;
    }
  }

  /** Closes the dialog: the scan stops and is dropped. */
  close() {
    this.#drop();
    this.#clear();
    this.roots = undefined;
    this.confirming = false;
  }

  /** Scans the two folders; the rows stream into the table. */
  async start(): Promise<void> {
    const roots = this.roots;
    if (roots === undefined || this.running) {
      return;
    }

    this.#drop();
    this.#clear();
    this.status = 'running';
    const run = this.#run;
    const id = await this.#launch(roots, run);

    if (id === undefined) {
      return;
    }

    if (run === this.#run) {
      this.#id = id;
      if (this.stopping) {
        this.#cancel(id);
      }
    } else {
      this.#cancel(id); // closed or started over before the answer
      this.#forget(id);
    }
  }

  /** Asks the backend for the scan: its id, or `undefined` when it refused (the reason is shown, unless the scan was dropped meanwhile). */
  async #launch(roots: Roots, run: number): Promise<CompareId | undefined> {
    const options = { subfolders: this.subfolders, content: this.content && this.contentAllowed, hidden: this.hidden };

    try {
      return await this.#ipc.invoke('compare_start', { ...roots, options, onEvent: (event) => this.#apply(run, event) });
    } catch (error) {
      if (run === this.#run) {
        this.status = 'idle';
        this.stopping = false;
        this.failure = IpcFailure.from(error).message;
      }

      return undefined;
    }
  }

  /** Stops the scan; the rows it found stay. */
  stop() {
    if (this.running && !this.stopping) {
      this.stopping = true;
      if (this.#id !== undefined) {
        this.#cancel(this.#id);
      }
    }
  }

  /** Sets the action of `entry`, when the row can go that way. */
  choose(entry: SyncRow, action: SyncAction): boolean {
    if (action !== 'skip' && whyNot(entry.row, action, this.#archives) !== undefined) {
      return false;
    }

    entry.action = action;
    this.#revision += 1;
    return true;
  }

  pick(entry: SyncRow) {
    entry.picked = !entry.picked;
    this.#revision += 1;
  }

  /** Picks (or lets go of) every row shown. */
  pickAll(picked: boolean) {
    for (const slot of this.#view) {
      this.#rows[slot].picked = picked;
    }

    this.#revision += 1;
  }

  /** Sets the action of the rows shown that are picked; the rows that cannot go that way keep theirs, and the notice says how many. */
  chooseAll(action: SyncAction) {
    let refused = 0;
    for (const slot of this.#view) {
      const entry = this.#rows[slot];
      if (entry.picked && !this.choose(entry, action)) {
        refused += 1;
      }
    }

    this.notice = refused === 0 ? undefined : `${refused.toLocaleString('en-US')} of the rows cannot go that way.`;
  }

  /** Asks for the confirmation of the copies; nothing is asked when there are none. */
  ask(): boolean {
    this.confirming = this.#totals.toRight.items + this.#totals.toLeft.items > 0;
    return this.confirming;
  }

  /** Queues the one job that makes the copies; a refusal rejects, so that the dialog can show it and stay. */
  async apply(): Promise<void> {
    const roots = this.roots;
    const items = this.items;
    if (roots !== undefined && items.length > 0) {
      await this.#ipc.invoke('ops_sync', { left: roots.left, right: roots.right, items });
    }
  }

  #apply(run: number, event: CompareEvent) {
    if (run !== this.#run) {
      return;
    }

    switch (event.kind) {
      case 'rows':
        for (const row of event.rows) {
          this.#rows.push({ row, action: defaultAction(row, this.#archives), picked: false });
          this.#counts[row.status] += 1;
        }
        break;
      case 'progress':
        this.progress = { folders: event.folders, files: event.files, bytes: event.bytes };
        return;
      case 'done':
        this.summary = { rows: event.rows, skipped: event.skipped, truncated: event.truncated, cancelled: event.cancelled };
        this.status = 'done';
        this.stopping = false;
        break;
      case 'error':
        this.failure = event.error.message;
        this.status = 'failed';
        this.stopping = false;
        break;
    }

    this.#revision += 1;
  }

  /** Cancels and forgets the dialog's scan; events of it that are still on their way are ignored. */
  #drop() {
    this.#run += 1;
    if (this.#id !== undefined) {
      if (this.running) {
        this.#cancel(this.#id);
      }

      this.#forget(this.#id);
    }

    this.#id = undefined;
  }

  #clear() {
    this.#rows = [];
    this.#counts = noCounts();
    this.#revision += 1;
    this.status = 'idle';
    this.stopping = false;
    this.progress = undefined;
    this.summary = undefined;
    this.failure = undefined;
    this.notice = undefined;
  }

  // A scan that is already gone (done and evicted, or the app is closing) needs nothing more done.
  #cancel(id: CompareId) {
    this.#ipc.invoke('compare_cancel', { id }).catch(() => {});
  }

  #forget(id: CompareId) {
    this.#ipc.invoke('compare_forget', { id }).catch(() => {});
  }
}
