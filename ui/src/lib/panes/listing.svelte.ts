import type { Crumb } from '$lib/ipc/bindings/Crumb';
import type { Entry } from '$lib/ipc/bindings/Entry';
import type { ListEvent } from '$lib/ipc/bindings/ListEvent';
import type { PaneId } from '$lib/ipc/bindings/PaneId';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

/** A listing slower than this shows a spinner in its pane; a faster one never flashes it. */
export const SPINNER_DELAY_MS = 150;

export type ListingStatus = 'loading' | 'ready' | 'error';

/**
 * One pane's folder as the backend streams it: `start` sizes the list, batches fill it in order, `done` adds the
 * summary. Only the latest `open` of a pane may change its state; events of an earlier one are dropped.
 */
export class Listing {
  path = $state<string | undefined>();
  parent = $state<string | null>(null);
  crumbs = $state.raw<Crumb[]>([]);
  /**
   * `total` slots; a slot is `undefined` until its batch arrives. A plain array filled in place (100 000 entries
   * must not be proxied, nor copied per batch); readers track `#revision`, bumped on every change.
   */
  #rows: (Entry | undefined)[] = [];
  #revision = $state(0);
  status = $state<ListingStatus>('loading');
  /** Set once loading has taken longer than {@link SPINNER_DELAY_MS}. */
  slow = $state(false);
  /** The folder cannot be shown at all (nothing listed before, or it failed partway). */
  error = $state<string | undefined>();
  /** Opening another folder failed; the current one stays listed and this says why. */
  notice = $state<string | undefined>();
  summary = $state<{ dirs: number; files: number; bytes: number } | undefined>();
  /** Milliseconds from `open` to the first batch (the first screen). */
  firstScreenMs = $state<number | undefined>();

  readonly #ipc: Transport;
  readonly #pane: PaneId;
  #showHidden = false;
  #generation = 0;
  #spinner: ReturnType<typeof setTimeout> | undefined;
  /** The folder changed while it was still loading: list it again once done. */
  #stale = false;
  /** Whether the latest open has delivered its `start`. */
  #started = false;

  get entries(): readonly (Entry | undefined)[] {
    void this.#revision;
    return this.#rows;
  }

  #setRows(rows: (Entry | undefined)[]) {
    this.#rows = rows;
    this.#revision += 1;
  }

  constructor(ipc: Transport, pane: PaneId) {
    this.#ipc = ipc;
    this.#pane = pane;
  }

  /** Opens `path` (`null`: the home folder), or its folder `child`. */
  open(path: string | null, child: string | null = null): Promise<void> {
    const generation = ++this.#generation;
    const started = performance.now();

    this.status = 'loading';
    this.notice = undefined;
    this.#stale = false;
    this.#started = false;
    clearTimeout(this.#spinner);
    this.slow = false;
    this.#spinner = setTimeout(() => {
      if (generation === this.#generation && this.status === 'loading') {
        this.slow = true;
      }
    }, SPINNER_DELAY_MS);

    const onEvent = (event: ListEvent) => {
      if (generation === this.#generation) {
        this.#apply(event, started);
      }
    };

    return this.#ipc
      .invoke('pane_open', { pane: this.#pane, path, child, showHidden: this.#showHidden, onEvent })
      .then(() => undefined)
      .catch((error: unknown) => {
        if (generation === this.#generation) {
          this.#fail(IpcFailure.from(error).message);
        }
      });
  }

  /** Lists the current folder again (after a change on disk, or with hidden entries toggled). */
  reload(): Promise<void> {
    return this.open(this.path ?? null);
  }

  setShowHidden(show: boolean): Promise<void> {
    this.#showHidden = show;
    return this.reload();
  }

  /** Opens the folder named `name` inside the current one. */
  enter(name: string): Promise<void> {
    return this.open(this.path ?? null, name);
  }

  #apply(event: ListEvent, started: number) {
    switch (event.kind) {
      case 'start': {
        // A reload of the same folder keeps showing the previous rows (and summary) until the new ones replace
        // them, so a live update does not flash empty rows.
        const reload = event.path === this.path && this.error === undefined;
        const next = new Array<Entry | undefined>(event.total);
        if (reload) {
          this.#rows.slice(0, event.total).forEach((entry, index) => (next[index] = entry));
        } else {
          this.summary = undefined;
        }
        this.#started = true;
        this.error = undefined;
        this.path = event.path;
        this.parent = event.parent;
        this.crumbs = event.crumbs;
        this.#setRows(next);
        this.firstScreenMs = undefined;
        if (event.total === 0) {
          this.firstScreenMs = performance.now() - started;
        }
        break;
      }
      case 'batch': {
        event.entries.forEach((entry, index) => (this.#rows[event.offset + index] = entry));
        this.#revision += 1;
        this.firstScreenMs ??= performance.now() - started;
        break;
      }
      case 'done':
        this.summary = { dirs: event.dirs, files: event.files, bytes: event.bytes };
        this.#settle('ready');
        if (this.#stale) {
          void this.reload();
        }
        break;
      case 'changed':
        if (this.status === 'loading') {
          this.#stale = true;
        } else {
          void this.reload();
        }
        break;
      case 'error':
        this.#fail(event.error.message);
        break;
    }
  }

  #fail(message: string) {
    // Another folder could not be opened: stay in the current one and say why.
    if (!this.#started && this.path !== undefined && this.error === undefined) {
      this.notice = message;
      this.#settle('ready');
      return;
    }

    this.error = message;
    this.#setRows([]);
    this.summary = undefined;
    this.#settle('error');
  }

  #settle(status: ListingStatus) {
    this.status = status;
    this.slow = false;
    clearTimeout(this.#spinner);
  }
}
