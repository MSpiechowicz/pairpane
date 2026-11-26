import type { PaneId } from '$lib/ipc/bindings/PaneId';
import type { SavedTab } from '$lib/ipc/bindings/SavedTab';
import type { SiteId } from '$lib/ipc/bindings/SiteId';
import type { Transport } from '$lib/ipc/transport';

import { extraTargetOf, gitPlaceOf } from './extras';
import { Extras } from './extras.svelte';
import type { Layout } from './layout';
import type { SortColumn } from './order';
import type { RemotePlace } from './remote';
import { Thumbs } from './thumbs.svelte';
import { PaneView } from './view.svelte';
import type { ViewHooks } from './viewHooks';

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
  /**
   * Where the tab was last left; `null`: the home folder. For the active tab the live folder is `view.listing.folder`.
   * While the tab is on a server it is the last folder on disk it was in: that is what is saved.
   */
  path: string | null;
  /** The folder on a server the tab is at, if it is. Only kept in memory: a connection does not outlive the app. */
  remote?: RemotePlace;
  sort: TabSort;
  layout: Layout;
  back: string[];
  forward: string[];
}

/** The folder, sort and layout of a tab as they are now (the active tab's live ones included). */
export interface TabState {
  path: string | null;
  sort: TabSort;
  layout: Layout;
}

/** How the pane got to the folder that is being opened, so that only real navigation is added to the history. */
type Move = 'back' | 'forward' | 'switch';

/**
 * One side's tabs. The side keeps a single {@link PaneView} (the backend lists and watches one folder per side), so
 * switching tabs re-opens the folder of the tab and restores its sort and layout. Each tab has its own back and forward
 * history; the history is recorded when a folder is listed, whoever asked for it (a click, the keyboard, a
 * crumb), except for the moves that walk the history itself and for tab switches.
 */
export class PaneTabs {
  readonly view: PaneView;
  /** The thumbnails the pane shows in its thumbnails layout; they outlive a change of folder or tab (a tab brings its folder back). */
  readonly thumbs: Thumbs;
  /** The values of the extra columns (dimensions, checksum, git status) the pane shows in its detailed layout; kept like the thumbnails. */
  readonly extras: Extras;

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

  constructor(ipc: Transport, pane: PaneId, hooks: Pick<ViewHooks, 'onFail' | 'showHidden'> = {}) {
    this.thumbs = new Thumbs(ipc);
    this.extras = new Extras(ipc);
    this.view = new PaneView(ipc, pane, {
      extraOf: (entry, column) => this.extras.sortKey(column, extraTargetOf(this.view.listing, entry), gitPlaceOf(this.view.listing, entry)),
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
      showHidden: hooks.showHidden,
    });
    this.#tabs = [this.#create(null, { column: 'name', desc: false }, 'detailed')];
  }

  #create(path: string | null, sort: TabSort, layout: Layout, remote?: RemotePlace): Tab {
    return { id: this.#nextId++, path, remote, sort, layout, back: [], forward: [] };
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

  /** The folders the tabs have been to and can go back or forward to, the active tab's first and the latest first. */
  get recent(): string[] {
    void this.#revision;
    const tabs = [this.#tabs[this.#active], ...this.#tabs.filter((_, index) => index !== this.#active)];

    return tabs.flatMap((tab) => [...tab.back].reverse().concat([...tab.forward].reverse()));
  }

  /** Every tab's folder, sort and layout, the active one's as shown now. Reactive. */
  get state(): TabState[] {
    return this.tabs.map((tab, index) =>
      index === this.#active ? this.#live(tab) : { path: tab.path, sort: tab.sort, layout: tab.layout },
    );
  }

  /** The folder on a server the active tab is at, as shown now (none while a move to a folder on disk is on its way). */
  #liveRemote(tab: Tab): RemotePlace | undefined {
    if (this.#entering) {
      return tab.remote;
    }

    return this.#route === undefined ? this.view.listing.remote : undefined;
  }

  /** The active tab as shown now. Its folder counts once it is listed: until then the tab's own path is the truth. */
  #live(tab: Tab): TabState {
    return {
      path: this.#entering ? tab.path : (this.view.listing.folder ?? tab.path),
      sort: { column: this.view.column, desc: this.view.desc },
      layout: this.view.layout,
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
    // A tab saved before layouts existed has none: it is shown in detail.
    const tabs = saved
      .slice(0, MAX_TABS)
      .map((tab) => this.#create(tab.path, { column: tab.sort, desc: tab.desc }, tab.layout ?? 'detailed'));
    if (tabs.length > 0) {
      this.#tabs = tabs;
      this.#active = Math.min(Math.max(active, 0), tabs.length - 1);
      this.#touch();
    }

    this.#enter(tabs.length > 0);
  }

  /**
   * The pane was refused the folder it was opening. If it was a tab's, the tab shows the home folder instead (once);
   * a place on a server that is gone is replaced by the last folder on disk the tab was in.
   */
  #refused() {
    this.#pending = undefined;
    this.#route = undefined;

    if (!this.#homing) {
      return;
    }

    const tab = this.#current;
    if (tab.remote !== undefined) {
      tab.remote = undefined;
      this.#go(tab.path, 'switch');
      return; // still homing: that folder may be refused too
    }

    this.#homing = false; // once: a home folder that is refused too is left as it is
    tab.path = null;
    this.#go(null, 'switch');
  }

  /**
   * A connection is over: the tabs that were on it are on their last folder on disk, the active one at once (its pane
   * shows that folder, which is no step of the history).
   */
  dropSite(site: SiteId) {
    for (const tab of this.#tabs) {
      if (tab.remote?.site === site) {
        tab.remote = undefined;
      }
    }

    if (this.view.listing.remote?.site === site) {
      this.#go(this.#current.path, 'switch');
      this.#entering = true;
      this.#homing = true;
    }
  }

  /** Adds a tab showing the current folder, after the active one, and makes it the active one. It copies the sort and layout. */
  add(): boolean {
    if (this.#tabs.length >= MAX_TABS) {
      return false;
    }

    this.#leave();
    const { path, sort, layout, remote } = this.#tabs[this.#active];
    this.#tabs.splice(this.#active + 1, 0, this.#create(path, { ...sort }, layout, remote));
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

  /** Makes the tab the active one: its folder is opened and its sort and layout applied. */
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
   * The active tab is about to stop being it: it keeps the sort and layout the pane shows and the folder it is at, or the one
   * a back or forward move not listed yet takes it to (that move's step goes into its history now).
   */
  #leave() {
    this.#settle();
    const tab = this.#current;
    const { path, sort, layout } = this.#live(tab);
    tab.path = this.#route ?? path;
    tab.remote = this.#liveRemote(tab);
    tab.sort = sort;
    tab.layout = layout;
  }

  /**
   * Shows the active tab: its sort and layout at once, its folder when it is listed. Not a step of its history. `home`: a
   * folder that cannot be opened is replaced by the home folder.
   */
  #enter(home: boolean) {
    const tab = this.#current;
    this.view.restoreSort(tab.sort.column, tab.sort.desc);
    this.view.layout = tab.layout;
    this.#go(tab.path, 'switch', undefined, tab.remote);
    this.#entering = true;
    this.#homing = home;
  }

  /**
   * Opens `path` (or the folder `remote` on a server) as a move that is not a navigation: the folder it lists is not
   * a new step of the history.
   */
  #go(path: string | null, move: Move, route?: string, remote?: RemotePlace) {
    void (remote === undefined ? this.view.listing.open(path) : this.view.listing.openRemote(remote));
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

  /**
   * The pane listed another folder: a step of the history, unless a history move or a tab switch brought it. A folder
   * on a server (`path` is `undefined`) is none: the history is of folders on disk, and the pane leaves from the last.
   */
  #listed(path: string | undefined) {
    const previous = this.#route ?? this.#shown;
    const move = this.#pending?.move;
    this.#homing = false;
    this.#entering = false;
    const tab = this.#current;
    this.#pending = undefined;
    this.#route = undefined;

    if (path === undefined) {
      return;
    }

    this.#shown = path;
    tab.path = path; // the folder on disk the tab was last in, whatever it shows next

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
