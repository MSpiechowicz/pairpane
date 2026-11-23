import { untrack } from 'svelte';

import { cancelSearch, forgetSearch } from '$lib/find/finder.svelte';
import { blankForm, toCriteria } from '$lib/find/query';
import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';
import type { FindEvent } from '$lib/ipc/bindings/FindEvent';
import type { FindId } from '$lib/ipc/bindings/FindId';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { isTyping } from '$lib/shell/shortcuts';

import type { Listing } from './listing.svelte';
import type { PaneTabs } from './tabs.svelte';

type Side = 'left' | 'right';

/**
 * `idle`: no flat view. `running`: the search walks the folder. `opening`: the walk is over and the pane is asked to
 * list its results. `shown`: the pane lists them.
 */
type Phase = 'idle' | 'running' | 'opening' | 'shown';

/** `12345` as `12 345`. */
const grouped = (count: number) => String(count).replace(/\B(?=(\d{3})+$)/g, ' ');

/**
 * One pane's flat view: a search for every file below the folder, whose results the pane then lists (as it lists the
 * results of the Find dialog). The search is the pane's own: it is dropped from the backend as soon as the pane lists
 * anything else (see {@link moved}), when another one replaces it, or when the walk is given up. The Find dialog's
 * search is another one and is left alone.
 */
export class FlatSide {
  #phase = $state<Phase>('idle');
  #id = $state<FindId | undefined>();
  /** The folder searched. */
  #root: string | undefined;
  /** What the pane listed when the walk started: another place while it runs means the pane has moved on. */
  #origin = '';
  /** Asked to stop; the search ends with its `done` shortly. */
  #stopping = false;
  #summary: { found: number; truncated: boolean; cancelled: boolean } | undefined;
  /** Tells the events of the latest search from those of one that was dropped. */
  #run = 0;

  readonly #ipc: Transport;
  readonly #listing: Listing;
  readonly #say: (text: string, error: boolean) => void;
  readonly #showHidden: () => boolean;

  constructor(ipc: Transport, listing: Listing, say: (text: string, error: boolean) => void, showHidden: () => boolean) {
    this.#ipc = ipc;
    this.#listing = listing;
    this.#say = say;
    this.#showHidden = showHidden;
  }

  /** The walk is going on. */
  get running(): boolean {
    return this.#phase === 'running';
  }

  /** The pane lists the results of this flat view (or is about to). */
  get shown(): boolean {
    const listed = this.#phase === 'opening' || this.#phase === 'shown';
    return listed && this.#id !== undefined && this.#listing.results === this.#id;
  }

  /** Ctrl+B: starts the walk; stops it while it runs; leads back to the folder from the results. */
  toggle() {
    const root = this.#root;

    if (this.#phase === 'running') {
      this.stop();
    } else if (this.#phase === 'shown' && this.#listing.results === this.#id && root !== undefined) {
      void this.#listing.open(root);
    } else if (this.#phase !== 'opening') {
      void this.#begin();
    }
  }

  /** Stops the walk; what it found is listed. */
  stop() {
    if (this.#phase !== 'running' || this.#stopping) {
      return;
    }

    this.#stopping = true;
    if (this.#id !== undefined) {
      cancelSearch(this.#ipc, this.#id);
    }
  }

  /** The pane listed another place (or the same one again): a flat view that is not shown any more is dropped. */
  moved(location: string) {
    const left = this.#phase === 'running' && location !== this.#origin;
    const gone = this.#phase === 'shown' && this.#listing.results !== this.#id;

    if (left || gone) {
      this.#drop();
    }
  }

  /** Why a flat view cannot be made of what the pane shows, or `undefined`. */
  #refusal(): string | undefined {
    if (this.#listing.archive !== undefined) {
      return 'A flat view lists a folder on disk: leave the archive first.';
    }

    if (this.#listing.results !== undefined) {
      return 'A flat view lists a folder, not search results: open the folder first.';
    }

    return this.#listing.folder === undefined ? 'Wait until the folder has been listed.' : undefined;
  }

  /** Every file below the folder; hidden ones as the pane shows them. */
  #criteria(): FindCriteria {
    // A size limit leaves out folders and links: with none that excludes anything, only files are left.
    return { ...(toCriteria(blankForm(), false) as FindCriteria), minSize: 0, skipHidden: !this.#showHidden() };
  }

  async #begin() {
    const refusal = this.#refusal();
    const root = this.#listing.folder;
    if (refusal !== undefined || root === undefined) {
      this.#say(refusal ?? 'Wait until the folder has been listed.', true);
      return;
    }

    this.#drop();
    const run = this.#run;
    this.#phase = 'running';
    this.#root = root;
    this.#origin = this.#listing.location;

    try {
      const id = await this.#ipc.invoke('find_start', {
        query: { root, criteria: this.#criteria() },
        onEvent: (event) => this.#apply(run, event),
      });

      if (run !== this.#run) {
        forgetSearch(this.#ipc, id); // given up before the answer
        return;
      }

      this.#id = id;
      if (this.#stopping) {
        cancelSearch(this.#ipc, id);
      }

      void this.#finish(run);
    } catch (error) {
      if (run === this.#run) {
        this.#drop();
        this.#say(IpcFailure.from(error).message, true);
      }
    }
  }

  #apply(run: number, event: FindEvent) {
    if (run !== this.#run) {
      return;
    }

    if (event.kind === 'done') {
      this.#summary = { found: event.found, truncated: event.truncated, cancelled: event.cancelled };
      void this.#finish(run);
    } else if (event.kind === 'error') {
      this.#drop();
      this.#say(event.error.message, true);
    }
  }

  /** Lists the results once the walk has ended and its id is known (either may come first). */
  async #finish(run: number) {
    const id = this.#id;
    const summary = this.#summary;
    if (id === undefined || summary === undefined || this.#phase !== 'running') {
      return;
    }

    // Nothing to list: a pane that shows no rows would only hide the folder. A walk that was stopped lists what it has.
    if (summary.found === 0) {
      this.#drop();
      this.#say(summary.cancelled ? 'Stopped before any file was found.' : 'There are no files below this folder.', false);
      return;
    }

    this.#phase = 'opening';
    await this.#listing.openResults(id);

    // The pane was sent elsewhere (or the results could not be listed) while they were being opened.
    if (run !== this.#run) {
      return;
    }

    if (this.#listing.results !== id) {
      this.#drop();
      return;
    }

    this.#phase = 'shown';
    if (summary.truncated) {
      this.#say(`Only the first ${grouped(summary.found)} files are listed.`, false);
    } else if (summary.cancelled) {
      this.#say(`Stopped: the ${grouped(summary.found)} files found so far are listed.`, false);
    }
  }

  /** Forgets the search (the backend stops it if it still runs); events on their way are ignored. */
  #drop() {
    this.#run += 1;
    if (this.#id !== undefined) {
      forgetSearch(this.#ipc, this.#id);
    }

    this.#id = undefined;
    this.#phase = 'idle';
    this.#stopping = false;
    this.#summary = undefined;
  }
}

/** The flat views of both panes. */
export class FlatView {
  readonly left: FlatSide;
  readonly right: FlatSide;
  readonly #listings: Record<Side, Listing>;

  constructor(
    ipc: Transport,
    panes: Record<Side, Pick<PaneTabs, 'view'>>,
    say: (text: string, error: boolean) => void,
    showHidden: () => boolean,
  ) {
    this.#listings = { left: panes.left.view.listing, right: panes.right.view.listing };
    this.left = new FlatSide(ipc, this.#listings.left, say, showHidden);
    this.right = new FlatSide(ipc, this.#listings.right, say, showHidden);
  }

  /**
   * Drops the flat view of a pane that has gone elsewhere. Starts effects: call while a component is being set up.
   */
  watch() {
    for (const side of ['left', 'right'] as const) {
      $effect(() => {
        const location = this.#listings[side].location;
        untrack(() => this[side].moved(location));
      });
    }
  }

  /** Esc stops the walk of the active pane (unless a text field has the key). */
  onKey(event: KeyboardEvent, active: Side) {
    if (event.key === 'Escape' && this[active].running && !isTyping(event.target)) {
      this[active].stop();
      event.preventDefault();
    }
  }
}
