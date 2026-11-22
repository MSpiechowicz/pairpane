import type { PaneId } from '$lib/ipc/bindings/PaneId';
import type { SavedTab } from '$lib/ipc/bindings/SavedTab';
import type { Transport } from '$lib/ipc/transport';

import type { SortColumn } from './order';
import { PaneView, type ViewHooks } from './view.svelte';

/** Tabs per pane. */
export const MAX_TABS = 32;
/** Entries per history stack (back, forward) of a tab. */
export const MAX_HISTORY = 50;

export interface TabSort {
  column: SortColumn;
  desc: boolean;
}

export interface Tab {
  readonly id: number;
  /** Where the tab was last left; `null`: the home folder. For the active tab the live folder is `view.listing.folder`. */
  path: string | null;
  sort: TabSort;
  back: string[];
  forward: string[];
}

/** The folder and sort of a tab as they are now (the active tab's live ones included). */
export interface TabState {
  path: string | null;
  sort: TabSort;
}

/** How the pane got to the folder that is being opened, so that only real navigation is added to the history. */
type Move = 'back' | 'forward' | 'switch';

/**
 * One side's tabs. The side keeps a single {@link PaneView} (the backend lists and watches one folder per side), so
 * switching tabs re-opens the folder of the tab and restores its sort. Each tab has its own back and forward
 * history; the history is recorded when a folder is listed, whoever asked for it (a click, the keyboard, a
 * crumb), except for the moves that walk the history itself and for tab switches.
 */
export class PaneTabs {
  readonly view: PaneView;

  /** Plain array (not proxied); readers track `#revision`, bumped on every change. */
  #tabs: Tab[];
  #revision = $state(0);
  #active = $state(0);
  #nextId = 1;

  /** The folder the pane listed last (the one a new move leaves behind). */
  #shown: string | undefined;
  /** Set by a move after its `open`; the listing it produces is not a new step. Any later open forgets it. */
  #pending: { path: string | null; move: Move } | undefined;
  /**
   * Where the history counts the pane to be while moves are stacked on one another: a move that follows one that
   * has not listed yet leaves from the folder the earlier one goes to, not from the one still shown.
   */
  #route: string | undefined;
  /** The folder of the active tab is being opened (a switch, a restore) and has not listed yet. */
  #entering = $state(false);
  /** A tab's folder is being opened; if it cannot be, the home folder takes its place (once). */
  #homing = false;

  constructor(ipc: Transport, pane: PaneId, hooks: Pick<ViewHooks, 'onFail'> = {}) {
    this.view = new PaneView(ipc, pane, {
      onOpen: () => {
        this.#pending = undefined;
        this.#route = undefined;
        // While a tab's folder has not listed, a move made meanwhile that is refused still falls back to home.
        if (!this.#entering) {
          this.#homing = false;
        }
      },
      onListed: (path) => this.#listed(path),
      onFail: () => {
        this.#refused();
        hooks.onFail?.();
      },
      onNotice: () => this.#refused(),
    });
    this.#tabs = [this.#create(null, { column: 'name', desc: false })];
  }

  #create(path: string | null, sort: TabSort): Tab {
    return { id: this.#nextId++, path, sort, back: [], forward: [] };
  }

  get tabs(): readonly Tab[] {
    void this.#revision;
    return this.#tabs;
  }

  get active(): number {
    return this.#active;
  }

  get canBack(): boolean {
    void this.#revision;
    return this.#tabs[this.#active].back.length > 0;
  }

  get canForward(): boolean {
    void this.#revision;
    return this.#tabs[this.#active].forward.length > 0;
  }

  /** Every tab's folder and sort, the active one's as shown now. Reactive. */
  get state(): TabState[] {
    return this.tabs.map((tab, index) =>
      index === this.#active ? this.#live(tab) : { path: tab.path, sort: tab.sort },
    );
  }

  /** The active tab as shown now. Its folder counts once it is listed: until then the tab's own path is the truth. */
  #live(tab: Tab): TabState {
    return {
      path: this.#entering ? tab.path : (this.view.listing.folder ?? tab.path),
      sort: { column: this.view.column, desc: this.view.desc },
    };
  }

  #touch() {
    this.#revision += 1;
  }

  // Tabs

  /**
   * Starts with the tabs of an earlier session (no history is kept) and shows the active one. Nothing usable saved
   * (no tabs) means the defaults: one tab in the home folder. A saved folder that cannot be opened is replaced by
   * the home folder.
   */
  restore(saved: readonly SavedTab[], active: number) {
    const tabs = saved.slice(0, MAX_TABS).map((tab) => this.#create(tab.path, { column: tab.sort, desc: tab.desc }));
    if (tabs.length > 0) {
      this.#tabs = tabs;
      this.#active = Math.min(Math.max(active, 0), tabs.length - 1);
      this.#touch();
    }

    this.#enter(tabs.length > 0);
  }

  /** The pane was refused the folder it was opening. If it was a tab's, the tab shows the home folder instead (once). */
  #refused() {
    this.#pending = undefined;
    this.#route = undefined;

    if (!this.#homing) {
      return;
    }

    this.#homing = false; // once: a home folder that is refused too is left as it is
    this.#current.path = null;
    this.#go(null, 'switch');
  }

  /** Adds a tab showing the current folder, after the active one, and makes it the active one. */
  add(): boolean {
    if (this.#tabs.length >= MAX_TABS) {
      return false;
    }

    this.#leave();
    const { path, sort } = this.#tabs[this.#active];
    this.#tabs.splice(this.#active + 1, 0, this.#create(path, { ...sort }));
    this.#active += 1;
    this.#touch();
    // A move of the tab it copies that has not listed yet is that tab's: the new one opens its folder itself, so
    // the move lists nowhere.
    if (this.#pending !== undefined) {
      this.#enter(true);
    }
    return true;
  }

  /** Closes a tab (the active one by default). The last tab stays. */
  close(index: number = this.#active): boolean {
    if (this.#tabs.length <= 1 || index < 0 || index >= this.#tabs.length) {
      return false;
    }

    if (index !== this.#active) {
      const current = this.#current;
      this.#tabs.splice(index, 1);
      this.#active = this.#tabs.indexOf(current);
      this.#touch();
      return true;
    }

    // The tab on its right takes its place (the one on its left when it was the last).
    this.#tabs.splice(index, 1);
    this.#active = Math.min(index, this.#tabs.length - 1);
    this.#touch();
    this.#enter(true);
    return true;
  }

  /** Makes the tab the active one: its folder is opened and its sort applied. */
  select(index: number) {
    if (index === this.#active || index < 0 || index >= this.#tabs.length) {
      return;
    }

    this.#leave();
    this.#active = index;
    this.#touch();
    this.#enter(true);
  }

  /** The next tab (`1`) or the previous one (`-1`), around the ends. */
  cycle(step: 1 | -1) {
    this.select((this.#active + step + this.#tabs.length) % this.#tabs.length);
  }

  get #current(): Tab {
    return this.#tabs[this.#active];
  }

  /**
   * The active tab is about to stop being it: it keeps the sort the pane shows and the folder it is at, or the one
   * a back or forward move not listed yet takes it to (that move's step goes into its history now).
   */
  #leave() {
    this.#settle();
    const tab = this.#current;
    const { path, sort } = this.#live(tab);
    tab.path = this.#route ?? path;
    tab.sort = sort;
  }

  /**
   * Shows the active tab: its sort at once, its folder when it is listed. Not a step of its history. `home`: a
   * folder that cannot be opened is replaced by the home folder.
   */
  #enter(home: boolean) {
    const tab = this.#current;
    this.view.restoreSort(tab.sort.column, tab.sort.desc);
    this.#go(tab.path, 'switch');
    this.#entering = true;
    this.#homing = home;
  }

  /** Opens `path` as a move that is not a navigation: the folder it lists is not a new step of the history. */
  #go(path: string | null, move: Move, route?: string) {
    void this.view.listing.open(path);
    // The open made the pane forget what it was doing; this is what it does now.
    this.#pending = { path, move };
    this.#route = route;
  }

  // History

  back() {
    this.#walk('back');
  }

  forward() {
    this.#walk('forward');
  }

  /**
   * Opens the folder on top of the stack the move reads from. It leaves the stack at once, so a folder that cannot
   * be opened any more is dropped from the history; the folder we leave goes on the other stack once the new one
   * is listed.
   */
  #walk(move: 'back' | 'forward') {
    this.#settle();
    const route = this.#route;
    const tab = this.#current;
    const target = (move === 'back' ? tab.back : tab.forward).pop();
    this.#touch();
    if (target === undefined) {
      return;
    }

    this.#go(target, move, route);
  }

  /**
   * A move that has not listed yet when another one follows counts as done: the folder the pane left for it goes
   * on the other stack now, so the history is what it would have been had the first one listed. The next move
   * then leaves from the folder the first one goes to.
   */
  #settle() {
    const pending = this.#pending;
    if (pending === undefined || pending.path === null) {
      return;
    }

    const tab = this.#current;
    const from = this.#route ?? this.#shown;
    if (from !== undefined && from !== pending.path) {
      if (pending.move === 'back') {
        remember(tab.forward, from);
      } else if (pending.move === 'forward') {
        remember(tab.back, from);
      }
    }

    this.#route = pending.path;
    this.#pending = { path: pending.path, move: 'switch' }; // its step is taken
  }

  /** The pane listed another folder: a step of the history, unless a history move or a tab switch brought it. */
  #listed(path: string) {
    const previous = this.#route ?? this.#shown;
    const move = this.#pending?.move;
    this.#homing = false;
    this.#entering = false;
    const tab = this.#current;
    this.#shown = path;
    this.#pending = undefined;
    this.#route = undefined;

    if (previous === undefined || previous === path) {
      return;
    }

    if (move === 'back') {
      remember(tab.forward, previous);
    } else if (move === 'forward') {
      remember(tab.back, previous);
    } else if (move === undefined) {
      remember(tab.back, previous);
      tab.forward = [];
    }

    this.#touch();
  }
}

/** Puts a folder on a history stack; the oldest entry goes when the stack is full. */
function remember(stack: string[], path: string) {
  stack.push(path);
  if (stack.length > MAX_HISTORY) {
    stack.shift();
  }
}
