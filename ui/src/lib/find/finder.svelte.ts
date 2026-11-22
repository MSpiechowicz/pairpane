import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';
import type { FindEvent } from '$lib/ipc/bindings/FindEvent';
import type { FindHit } from '$lib/ipc/bindings/FindHit';
import type { FindId } from '$lib/ipc/bindings/FindId';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { blankForm, type FindForm } from './query';

/** A row of the results: a result, or the head of a group of duplicates (its results follow it). */
export type FindRow = { kind: 'hit'; hit: FindHit } | { kind: 'group'; size: number; count: number };

export type FindStatus = 'idle' | 'running' | 'done' | 'failed';

export interface FindSummary {
  found: number;
  skipped: number;
  truncated: boolean;
  cancelled: boolean;
}

/** Where the results of a search can be listed: a pane. */
export interface ResultsTarget {
  openResults(id: FindId): Promise<void>;
}

/**
 * The Find dialog's state: its fields (kept when it is closed and opened again), the search that runs and its
 * results as they stream in. At most one search is the dialog's: starting another, or closing the dialog, drops it from
 * the backend. A search whose results a pane lists is the pane's from then on (the pane lists them again on a reload):
 * it is left alone for as long as a pane shows it, and dropped at the next start or close once none does.
 */
export class Finder {
  readonly form = $state<FindForm>(blankForm());
  /** The folder searched; `undefined` while the dialog is closed. */
  root = $state<string | undefined>();
  status = $state<FindStatus>('idle');
  /** Asked to stop; the search ends with its `done` shortly. */
  stopping = $state(false);
  progress = $state<{ folders: number; files: number; bytes: number } | undefined>();
  summary = $state<FindSummary | undefined>();
  failure = $state<string | undefined>();

  readonly #ipc: Transport;
  /** A plain array filled in place (100 000 results must not be proxied); readers track `#revision`. */
  #rows: FindRow[] = [];
  #revision = $state(0);
  #hits = $state(0);
  #inside = $state(0);
  #id: FindId | undefined;
  #fed = false;
  /** Searches that were listed in a pane and are not dropped yet. */
  readonly #listed = new Set<FindId>();
  /** Whether a pane shows the results of the search. */
  readonly #shown: (id: FindId) => boolean;
  /** Tells the events of the latest search from those of one that was dropped. */
  #run = 0;

  constructor(ipc: Transport, shown: (id: FindId) => boolean = () => false) {
    this.#ipc = ipc;
    this.#shown = shown;
  }

  /** The dialog is open and has the keyboard. */
  get modal(): boolean {
    return this.root !== undefined;
  }

  get rows(): readonly FindRow[] {
    void this.#revision;
    return this.#rows;
  }

  /** Results so far. */
  get hits(): number {
    return this.#hits;
  }

  /** Results inside archives so far: they are not listed in a pane. */
  get inside(): number {
    return this.#inside;
  }

  get running(): boolean {
    return this.status === 'running';
  }

  /** The results can be listed in a pane: a search that is not running has the filesystem's results. */
  get feedable(): boolean {
    return this.#id !== undefined && !this.running && this.#hits > this.#inside;
  }

  /** Opens the dialog on `root`, with no results. */
  show(root: string) {
    if (this.root === undefined) {
      this.#clear();
      this.root = root;
    }
  }

  /** Closes the dialog: the search stops and its results are dropped, unless a pane lists them. */
  close() {
    this.#drop();
    this.#clear();
    this.root = undefined;
  }

  /** Why the form cannot be searched (found out before anything is asked). */
  refuse(message: string) {
    this.failure = message;
  }

  /** Searches the folder for `criteria`; the results stream into {@link rows}. */
  async start(criteria: FindCriteria): Promise<void> {
    if (this.root === undefined || this.running) {
      return;
    }

    this.#drop();
    this.#clear();
    this.status = 'running';
    const run = this.#run;

    try {
      const id = await this.#ipc.invoke('find_start', {
        query: { root: this.root, criteria },
        onEvent: (event) => this.#apply(run, event),
      });

      if (run !== this.#run) {
        this.#forget(id); // closed or started over before the answer
      } else {
        this.#id = id;
        if (this.stopping) {
          this.#cancel(id);
        }
      }
    } catch (error) {
      if (run === this.#run) {
        this.status = 'idle';
        this.stopping = false;
        this.failure = IpcFailure.from(error).message;
      }
    }
  }

  /** Stops the search; what it found stays. */
  stop() {
    if (!this.running || this.stopping) {
      return;
    }

    this.stopping = true;
    if (this.#id !== undefined) {
      this.#cancel(this.#id);
    }
  }

  /** Lists the results in `target`. They are the pane's then: closing the dialog does not drop them. */
  async feed(target: ResultsTarget): Promise<boolean> {
    const id = this.#id;
    if (id === undefined || !this.feedable) {
      return false;
    }

    this.#fed = true;
    try {
      await target.openResults(id);
      return true;
    } catch (error) {
      this.#fed = false;
      this.failure = IpcFailure.from(error).message;
      return false;
    }
  }

  #apply(run: number, event: FindEvent) {
    if (run !== this.#run) {
      return;
    }

    switch (event.kind) {
      case 'found':
        this.#add(event.hits);
        break;
      case 'group':
        this.#rows.push({ kind: 'group', size: event.size, count: event.hits.length });
        this.#add(event.hits);
        break;
      case 'progress':
        this.progress = { folders: event.folders, files: event.files, bytes: event.bytes };
        return;
      case 'done':
        this.summary = {
          found: event.found,
          skipped: event.skipped,
          truncated: event.truncated,
          cancelled: event.cancelled,
        };
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

  #add(hits: readonly FindHit[]) {
    for (const hit of hits) {
      this.#rows.push({ kind: 'hit', hit });
      this.#inside += hit.inner === null ? 0 : 1;
    }

    this.#hits += hits.length;
  }

  /**
   * Forgets the dialog's search, unless a pane lists it, and the searches listed earlier that no pane shows any more;
   * events of the dropped search that are still on their way are ignored.
   */
  #drop() {
    this.#run += 1;
    if (this.#id !== undefined) {
      if (this.#fed) {
        this.#listed.add(this.#id);
      } else {
        this.#forget(this.#id);
      }
    }

    this.#id = undefined;
    this.#fed = false;

    for (const id of this.#listed) {
      if (!this.#shown(id)) {
        this.#listed.delete(id);
        this.#forget(id);
      }
    }
  }

  #clear() {
    this.#rows = [];
    this.#revision += 1;
    this.#hits = 0;
    this.#inside = 0;
    this.status = 'idle';
    this.stopping = false;
    this.progress = undefined;
    this.summary = undefined;
    this.failure = undefined;
  }

  // A search that is already gone (evicted, or the app is closing) needs nothing more done.
  #cancel(id: FindId) {
    this.#ipc.invoke('find_cancel', { id }).catch(() => {});
  }

  #forget(id: FindId) {
    this.#ipc.invoke('find_forget', { id }).catch(() => {});
  }
}
