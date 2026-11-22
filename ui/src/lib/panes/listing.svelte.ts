import type { ArchiveSpot } from '$lib/ipc/bindings/ArchiveSpot';
import type { Crumb } from '$lib/ipc/bindings/Crumb';
import type { Entry } from '$lib/ipc/bindings/Entry';
import type { ListEvent } from '$lib/ipc/bindings/ListEvent';
import type { PaneId } from '$lib/ipc/bindings/PaneId';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { joinInner, sameSpot } from './archive';

/** A listing slower than this shows a spinner in its pane; a faster one never flashes it. */
export const SPINNER_DELAY_MS = 150;

export type ListingStatus = 'loading' | 'ready' | 'error';

/** What an open asks for: a real folder (`null`: home) and one of its folders, or a place inside an archive. */
type Target = { folder: string | null; child: string | null } | { spot: ArchiveSpot };

/** What a pane's view needs to hear about a listing besides its (reactive) state. */
export interface ListingHooks {
  /** An open begins (any open, a silent one included), before any of its events. */
  onOpen?: () => void;
  /** A listing started. `reload`: the same folder again (a change on disk, hidden toggled), not another folder. */
  onStart?: (reload: boolean) => void;
  /** A listing delivered its last batch. */
  onDone?: () => void;
  /** An open failed and the pane has nothing to show (a failed move away from a listed folder is a notice instead). */
  onFail?: () => void;
  /** An open of another folder failed and the pane stays in the folder it had (the notice says why). */
  onNotice?: () => void;
}

/**
 * One pane's folder as the backend streams it: `start` sizes the list, batches fill it in order, `done` adds the
 * summary. Only the latest `open` of a pane may change its state; events of an earlier one are dropped. A folder
 * inside an archive is listed the same way (`openArchive`); the archive is then read-only and has no `path`.
 */
export class Listing {
  /** The real folder listed, or the folder that holds the archive shown. */
  #folder = $state<string | undefined>();
  /** Set while the pane shows a folder inside an archive. */
  archive = $state.raw<ArchiveSpot | undefined>();
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
  summary = $state<{ dirs: number; files: number; bytes: number; free: number | null } | undefined>();
  /** Milliseconds from `open` to the first batch (the first screen). */
  firstScreenMs = $state<number | undefined>();

  readonly #ipc: Transport;
  readonly #pane: PaneId;
  readonly #hooks: ListingHooks;
  #showHidden = false;
  #generation = 0;
  #spinner: ReturnType<typeof setTimeout> | undefined;
  /** The folder changed while it was still loading: list it again once done. */
  #stale = false;
  /** Whether the latest open has delivered its `start`. */
  #started = false;

  /** The real folder shown; `undefined` before the first listing and inside an archive (nothing there is a path). */
  get path(): string | undefined {
    return this.archive === undefined ? this.#folder : undefined;
  }

  /** The real folder shown, or inside an archive the folder that holds it. */
  get folder(): string | undefined {
    return this.#folder;
  }

  /** What is listed cannot be changed: it is inside an archive. */
  get readonly(): boolean {
    return this.archive !== undefined;
  }

  /** Tells one listed place from another: a folder, or a folder inside an archive. */
  get location(): string {
    return this.archive === undefined ? (this.#folder ?? '') : `${this.archive.archive}\0${this.archive.dir}`;
  }

  get entries(): readonly (Entry | undefined)[] {
    void this.#revision;
    return this.#rows;
  }

  #setRows(rows: (Entry | undefined)[]) {
    this.#rows = rows;
    this.#revision += 1;
  }

  constructor(ipc: Transport, pane: PaneId, hooks: ListingHooks = {}) {
    this.#ipc = ipc;
    this.#pane = pane;
    this.#hooks = hooks;
  }

  /** Opens `path` (`null`: the home folder), or its folder `child`. */
  open(path: string | null, child: string | null = null): Promise<void> {
    return this.#open({ folder: path, child }, false);
  }

  /** Opens the folder of an archive `spot`, or its folder `child`. */
  openArchive(spot: ArchiveSpot, child: string | null = null): Promise<void> {
    const dir = child === null ? spot.dir : joinInner(spot.dir, child);
    return this.#open({ spot: { archive: spot.archive, dir } }, false);
  }

  /** What is shown now, as an open that lists it again. */
  #here(): Target {
    return this.archive === undefined ? { folder: this.#folder ?? null, child: null } : { spot: this.archive };
  }

  /** Whether `target` is the place shown (a reload), not another one. */
  #isHere(target: Target): boolean {
    if ('spot' in target) {
      return sameSpot(target.spot, this.archive);
    }

    return this.archive === undefined && target.child === null && target.folder === (this.#folder ?? null);
  }

  #request(target: Target, onEvent: (event: ListEvent) => void) {
    const showHidden = this.#showHidden;
    if ('spot' in target) {
      const { archive, dir } = target.spot;
      return this.#ipc.invoke('archive_open', {
        pane: this.#pane,
        archive,
        dir,
        showHidden,
        onEvent,
      });
    }

    const { folder, child } = target;
    return this.#ipc.invoke('pane_open', {
      pane: this.#pane,
      path: folder,
      child,
      showHidden,
      onEvent,
    });
  }

  /**
   * `quiet`: lists the shown folder again behind a failed open, only so the backend watches it once more; the
   * notice stays and the pane does not show itself as loading.
   */
  #open(target: Target, quiet: boolean): Promise<void> {
    const generation = ++this.#generation;
    const started = performance.now();
    // Listing the shown folder again (a live update, hidden toggled) must not be answered by one more reload.
    const reload = this.#isHere(target);

    this.#hooks.onOpen?.();
    this.#stale = false;
    this.#started = false;

    if (!quiet) {
      this.status = 'loading';
      this.notice = undefined;
      clearTimeout(this.#spinner);
      this.slow = false;
      this.#spinner = setTimeout(() => {
        if (generation === this.#generation && this.status === 'loading') {
          this.slow = true;
        }
      }, SPINNER_DELAY_MS);
    }

    const onEvent = (event: ListEvent) => {
      if (generation === this.#generation) {
        this.#apply(event, started, reload);
      }
    };

    return this.#request(target, onEvent)
      .then(() => undefined)
      .catch((error: unknown) => {
        if (generation === this.#generation) {
          this.#fail(IpcFailure.from(error).message, reload);
        }
      });
  }

  /** Lists the current folder again (after a change on disk, or with hidden entries toggled). */
  reload(): Promise<void> {
    return this.#open(this.#here(), false);
  }

  setShowHidden(show: boolean): Promise<void> {
    this.#showHidden = show;
    return this.reload();
  }

  /** Opens the folder named `name` inside the current one (inside an archive too). */
  enter(name: string): Promise<void> {
    return this.archive === undefined ? this.open(this.#folder ?? null, name) : this.openArchive(this.archive, name);
  }

  #apply(event: ListEvent, started: number, wasReload: boolean) {
    switch (event.kind) {
      case 'start': {
        // A reload of the same folder keeps showing the previous rows (and summary) until the new ones replace
        // them, so a live update does not flash empty rows.
        const reload = event.path === this.#folder && sameSpot(event.archive, this.archive) && this.error === undefined;
        const next = new Array<Entry | undefined>(event.total);
        if (reload) {
          this.#rows.slice(0, event.total).forEach((entry, index) => (next[index] = entry));
        } else {
          this.summary = undefined;
        }
        this.#started = true;
        this.error = undefined;
        this.#folder = event.path;
        this.archive = event.archive;
        this.parent = event.parent;
        this.crumbs = event.crumbs;
        this.#setRows(next);
        this.firstScreenMs = undefined;
        if (event.total === 0) {
          this.firstScreenMs = performance.now() - started;
        }
        this.#hooks.onStart?.(reload);
        break;
      }
      case 'batch': {
        event.entries.forEach((entry, index) => (this.#rows[event.offset + index] = entry));
        this.#revision += 1;
        this.firstScreenMs ??= performance.now() - started;
        break;
      }
      case 'done':
        this.summary = {
          dirs: event.dirs,
          files: event.files,
          bytes: event.bytes,
          free: event.free,
        };
        this.#settle('ready');
        this.#hooks.onDone?.();
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
        this.#fail(event.error.message, wasReload);
        break;
    }
  }

  /** `wasReload`: the failed open was of the folder shown (nothing to go back to, or it would never end). */
  #fail(message: string, wasReload: boolean) {
    // Another folder could not be opened: stay in the current one and say why. The failed open made the backend
    // stop watching the shown folder, so it is listed again quietly to bring the watch back.
    if (!this.#started && this.#folder !== undefined && this.error === undefined) {
      this.notice = message;
      this.#settle('ready');

      // The owner may open another folder at once; that open brings the watch back by itself.
      const generation = this.#generation;
      this.#hooks.onNotice?.();
      if (!wasReload && generation === this.#generation) {
        void this.#open(this.#here(), true);
      }

      return;
    }

    this.error = message;
    this.#setRows([]);
    this.summary = undefined;
    this.#settle('error');
    this.#hooks.onFail?.();
  }

  #settle(status: ListingStatus) {
    this.status = status;
    this.slow = false;
    clearTimeout(this.#spinner);
  }
}
