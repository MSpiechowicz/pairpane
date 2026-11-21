import { tick } from 'svelte';

import type { PaneId } from '$lib/ipc/bindings/PaneId';
import type { Transport } from '$lib/ipc/transport';
import { focusActiveList } from '$lib/panes/focus';
import type { PaneTabs } from '$lib/panes/tabs.svelte';

import { ViewSession } from './session.svelte';

/** How long the cursor rests on a file before the quick view opens it (holding an arrow key opens nothing). */
export const FOLLOW_DELAY_MS = 150;

/** A file under a pane's cursor. */
export interface Target {
  path: string;
  name: string;
}

/** What the active pane's cursor is on: a file, a folder (`..` too) or nothing that can be shown yet. */
export type Under = Target | 'folder' | undefined;

/** What the quick view shows: a session, the placeholder for a folder, or nothing. */
export type QuickShown = ViewSession | 'folder' | undefined;

const sameTarget = (a: Under, b: Under) =>
  a === b || (typeof a === 'object' && typeof b === 'object' && a.path === b.path && a.name === b.name);

/**
 * The viewer of the window: the full overlay F3 opens for the file under the active cursor, and the quick view
 * (Ctrl+Q) that takes the place of the other pane and shows what the active cursor is on. Each shows a
 * {@link ViewSession}; the one that is left is closed on the backend.
 */
export class Viewer {
  /** The overlay's session (F3). */
  overlay = $state.raw<ViewSession | undefined>();
  quick = $state(false);
  quickShown = $state.raw<QuickShown>();

  /** What the active pane's cursor is on. */
  readonly under: Under = $derived.by(() => this.#under());

  readonly #ipc: Transport;
  readonly #panes: Record<PaneId, PaneTabs>;
  readonly #active: () => PaneId;
  #timer: ReturnType<typeof setTimeout> | undefined;
  /** What the quick view shows or is about to show. */
  #pending: Under;

  constructor(ipc: Transport, panes: Record<PaneId, PaneTabs>, active: () => PaneId) {
    this.#ipc = ipc;
    this.#panes = panes;
    this.#active = active;
  }

  /** The overlay has the keyboard: the shortcuts and Tab are its own then. */
  get modal(): boolean {
    return this.overlay !== undefined;
  }

  #under(): Under {
    const view = this.#panes[this.#active()].view;
    const path = view.listing.path;
    if (path === undefined) {
      return undefined;
    }

    const entry = view.entryAt(view.cursor);
    if (entry === undefined) {
      return view.isUp(view.cursor) ? 'folder' : undefined;
    }

    return entry.kind === 'dir' ? 'folder' : { path, name: entry.name };
  }

  /** F3: the overlay for the file under the cursor. A folder, `..` and an empty pane have nothing to view. */
  open() {
    const target = this.under;
    if (typeof target === 'object') {
      this.openAt(target);
    }
  }

  /** The overlay for `target`, with a word on why it is shown when it was not asked for (the editor sent it). */
  openAt(target: Target, notice?: string) {
    if (this.overlay === undefined) {
      this.overlay = new ViewSession(this.#ipc, target.path, target.name, notice);
    }
  }

  /** Closes the overlay; the list that had the focus takes it back (the overlay's dialog gives it back too). */
  close() {
    this.overlay?.close();
    this.overlay = undefined;
    this.#settle();
  }

  /** Ctrl+Q: the other pane becomes the quick view, or the pane comes back. */
  toggleQuick() {
    this.quick = !this.quick;
    if (!this.quick) {
      this.#drop();
      this.#settle();
    }
  }

  /**
   * Shows `target` in the quick view: at once when nothing is shown yet, else when the cursor has stayed on it for
   * {@link FOLLOW_DELAY_MS}. The same file again changes nothing, so a listing that reloads keeps the view.
   */
  follow(target: Under) {
    clearTimeout(this.#timer);
    if (!this.quick || sameTarget(target, this.#pending)) {
      return;
    }

    if (this.quickShown === undefined) {
      this.#show(target);
    } else {
      this.#timer = setTimeout(() => this.#show(target), FOLLOW_DELAY_MS);
    }
  }

  /** The quick view's pane went away (or the cursor moved on): a wait that is not over is over. */
  unfollow() {
    clearTimeout(this.#timer);
  }

  #show(target: Under) {
    this.#pending = target;
    const earlier = this.quickShown;
    this.quickShown = typeof target === 'object' ? new ViewSession(this.#ipc, target.path, target.name) : target;
    if (earlier instanceof ViewSession) {
      earlier.close();
    }
  }

  #drop() {
    clearTimeout(this.#timer);
    const earlier = this.quickShown;
    this.quickShown = undefined;
    this.#pending = undefined;
    if (earlier instanceof ViewSession) {
      earlier.close();
    }
  }

  #settle() {
    void tick().then(focusActiveList);
  }
}
