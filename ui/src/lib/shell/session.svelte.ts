import { untrack } from 'svelte';

import type { PaneId } from '$lib/ipc/bindings/PaneId';
import type { SavedPane } from '$lib/ipc/bindings/SavedPane';
import type { SavedState } from '$lib/ipc/bindings/SavedState';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import type { SavedSearches } from '$lib/find/saved.svelte';
import type { Hotlist } from '$lib/panes/hotlist.svelte';
import { savedLayout } from '$lib/panes/layout';
import { savedSort } from '$lib/panes/order';
import type { PaneTabs } from '$lib/panes/tabs.svelte';
import type { SavedRenames } from '$lib/rename/saved.svelte';

import type { Keymap } from './keymap.svelte';
import type { Preferences } from './preferences.svelte';

/** The load is given this long; a config folder on a dead share must not keep the panes empty. */
export const LOAD_TIMEOUT_MS = 2000;
/** A change is saved this long after the last one, so a burst of navigation is one write. */
export const SAVE_DELAY_MS = 400;
/** A save the backend refused as busy is tried again after this long, with the latest state. */
export const BUSY_SAVE_MS = 500;

/** What the load gives when it cannot say what is saved. */
const UNKNOWN = Symbol('unknown');

export interface SessionParts {
  panes: Record<PaneId, PaneTabs>;
  hotlist: Hotlist;
  searches: SavedSearches;
  renames: SavedRenames;
  /** The keys the user set for actions. */
  keymap: Keymap;
  /** What the settings set. */
  preferences: Preferences;
  /** The pane that has the keyboard focus. */
  active: { get: () => PaneId; set: (pane: PaneId) => void };
}

/**
 * What survives a restart: both panes' tabs (folder, sort and layout), their active tabs, the active pane and the
 * hotlist, the saved searches, the saved rename rules, the keys the user set and the preferences. {@link restore} runs once at startup, before any folder is opened; {@link watch} then saves every change
 * a moment after it, and once more when the page goes away.
 */
export class Session {
  readonly #ipc: Transport;
  readonly #parts: SessionParts;

  /**
   * Saving starts only after the restore, so the defaults never replace what was saved; and never when the saved
   * state could not be read (too slow, or refused), since what is on disk is then unknown and may be worth keeping.
   */
  #ready = $state(false);
  /** The latest state seen and the text of the one the backend has. */
  #latest: { state: SavedState; text: string } | undefined;
  #saved: string | undefined;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #saving = false;
  #reported = false;

  constructor(ipc: Transport, parts: SessionParts) {
    this.#ipc = ipc;
    this.#parts = parts;
  }

  /**
   * Brings back the saved state, or the defaults (both panes in the home folder), and opens the folders. Whether the
   * state is known: `false` when it could not be read (or took too long), when what the user set is not known either.
   */
  async restore(): Promise<boolean> {
    const loaded = await this.#load();
    const saved = loaded === UNKNOWN ? null : loaded;
    const { panes, hotlist, searches, renames, keymap, preferences, active } = this.#parts;

    // First: the folders are listed as the preferences say (with or without hidden files).
    preferences.restore(saved?.preferences);

    for (const side of ['left', 'right'] as const) {
      const pane: SavedPane = saved?.[side] ?? { tabs: [], active: 0 };
      panes[side].restore(pane.tabs, pane.active);
    }

    hotlist.restore(saved?.hotlist ?? []);
    searches.restore(saved?.searches ?? []);
    renames.restore(saved?.renames ?? []);
    keymap.restore(saved?.keys ?? []);
    if (saved !== null) {
      active.set(saved.active);
      // What was loaded is what the backend has: starting up is not a change to save.
      this.#saved = this.#snapshot()?.text;
    }

    this.#ready = loaded !== UNKNOWN;

    return this.#ready;
  }

  /** `null` when nothing (usable) is saved; {@link UNKNOWN} when the load failed or takes too long. */
  async #load(): Promise<SavedState | null | typeof UNKNOWN> {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tooSlow = new Promise<typeof UNKNOWN>((resolve) => {
      timer = setTimeout(() => resolve(UNKNOWN), LOAD_TIMEOUT_MS);
    });

    try {
      return await Promise.race([this.#ipc.invoke('state_load', undefined), tooSlow]);
    } catch {
      return UNKNOWN;
    } finally {
      clearTimeout(timer);
    }
  }

  /**
   * Saves changes. Call while a component is being set up: it starts an effect that follows the state, and the
   * listener that saves at once when the page goes away (best effort: the page may not live to see the answer).
   */
  watch() {
    $effect(() => {
      const next = this.#snapshot();
      if (this.#ready && next !== undefined) {
        untrack(() => this.#changed(next));
      }
    });

    $effect(() => {
      const flush = () => this.#flush();
      window.addEventListener('pagehide', flush);
      return () => {
        window.removeEventListener('pagehide', flush);
        clearTimeout(this.#timer);
      };
    });
  }

  /** The state as it is now; `undefined` while a folder has not been listed yet (there is no path to save). */
  #snapshot(): { state: SavedState; text: string } | undefined {
    const { panes, hotlist, searches, renames, keymap, preferences, active } = this.#parts;
    const left = this.#pane(panes.left);
    const right = this.#pane(panes.right);
    if (left === undefined || right === undefined) {
      return undefined;
    }

    // Always sent, even when there are none: a save without them would leave what the backend has, and a delete would not stick.
    const state: SavedState = {
      active: active.get(),
      left,
      right,
      hotlist: [...hotlist.paths],
      searches: [...searches.list],
      renames: [...renames.list],
      keys: keymap.overrides,
      preferences: { ...preferences.value },
    };
    return { state, text: JSON.stringify(state) };
  }

  #pane(tabs: PaneTabs): SavedPane | undefined {
    const saved = [];
    for (const { path, sort, layout } of tabs.state) {
      if (path === null) {
        return undefined;
      }

      // The detailed layout is the one a tab has when none is saved.
      saved.push({ path, ...savedSort(sort.column, sort.desc), ...(layout === 'detailed' ? {} : { layout: savedLayout(layout) }) });
    }

    return { tabs: saved, active: tabs.active };
  }

  #changed(next: { state: SavedState; text: string }) {
    this.#latest = next;
    clearTimeout(this.#timer);

    if (next.text !== this.#saved) {
      this.#timer = setTimeout(() => this.#flush(), SAVE_DELAY_MS);
    }
  }

  /** Writes the latest state, unless the backend has it already or a write is still on its way. */
  #flush() {
    clearTimeout(this.#timer);
    const latest = this.#latest;
    if (this.#saving || latest === undefined || latest.text === this.#saved) {
      return;
    }

    this.#saving = true;
    this.#ipc
      .invoke('state_save', { state: latest.state })
      .then(() => {
        this.#saved = latest.text;
        this.#saving = false;
        this.#flush(); // a change made while this write ran
      })
      .catch((error: unknown) => {
        this.#saving = false;
        this.#failed(IpcFailure.from(error));
      });
  }

  #failed(failure: IpcFailure) {
    if (failure.code === 'busy') {
      this.#timer = setTimeout(() => this.#flush(), BUSY_SAVE_MS);
      return;
    }

    // Not a thing a retry fixes (a refused state, a disk that cannot be written): the next change tries again.
    if (!this.#reported) {
      this.#reported = true;
      console.warn(`The session could not be saved (${failure.code}): ${failure.message}`);
    }
  }
}
