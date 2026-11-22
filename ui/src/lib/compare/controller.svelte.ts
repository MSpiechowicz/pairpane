import type { CompareRoot } from '$lib/ipc/bindings/CompareRoot';
import type { PaneId } from '$lib/ipc/bindings/PaneId';
import type { Transport } from '$lib/ipc/transport';
import { isMacPlatform, isWindowsPlatform } from '$lib/panes/keys';
import type { PaneTabs } from '$lib/panes/tabs.svelte';
import type { PaneView } from '$lib/panes/view.svelte';
import { RESULTS_NOTICE } from '$lib/ops/targets';

import { compareListings, quickSummary } from './quick';
import { Syncer } from './syncer.svelte';

/** The names of a folder fold together (`A.txt` is `a.TXT`) where the filesystem does not tell cases apart. */
const foldsCase = () => isWindowsPlatform() || isMacPlatform();

/** What a pane shows as one side of a comparison: its folder, or the folder inside its archive; search results are neither. */
function rootOf(view: PaneView): CompareRoot | undefined {
  const { archive, path } = view.listing;
  if (archive !== undefined) {
    return { kind: 'archive', archive: archive.archive, dir: archive.dir };
  }

  return path === undefined ? undefined : { kind: 'folder', path };
}

/** Why the two panes cannot be compared, or `undefined` when they can. */
function refusal(left: PaneView, right: PaneView): string | undefined {
  if (left.listing.results !== undefined || right.listing.results !== undefined) {
    return RESULTS_NOTICE;
  }

  if (left.listing.status === 'loading' || right.listing.status === 'loading') {
    return 'Wait until the folders have been listed.';
  }

  if (left.listing.status === 'error' || right.listing.status === 'error') {
    return 'A folder could not be listed.';
  }

  return left.listing.location === right.listing.location ? 'Both panes show the same folder.' : undefined;
}

/**
 * Compares the two panes' folders: the quick comparison of what they list now (it marks and selects the
 * differences), and the Synchronize dialog's scan of the folders and everything below them. A refusal is told in a
 * card, `say`.
 */
export class Comparison {
  readonly syncer: Syncer;

  readonly #panes: Record<PaneId, PaneTabs>;
  readonly #say: (text: string, error: boolean) => void;
  /** Whether the panes list hidden files. */
  readonly #hidden: () => boolean;

  constructor(ipc: Transport, panes: Record<PaneId, PaneTabs>, say: (text: string, error: boolean) => void, hidden: () => boolean) {
    this.syncer = new Syncer(ipc);
    this.#panes = panes;
    this.#say = say;
    this.#hidden = hidden;
  }

  /** A dialog has the keyboard: the shortcuts and Tab are its own then. */
  get modal(): boolean {
    return this.syncer.modal;
  }

  run(choice: 'compare' | 'synchronize') {
    if (choice === 'compare') {
      this.quick();
    } else {
      this.synchronize();
    }
  }

  /** Marks what differs in the two panes and selects, on each side, what is only there or newer there (what F5 copies over). */
  quick() {
    const { left, right } = { left: this.#panes.left.view, right: this.#panes.right.view };
    const refused = refusal(left, right);
    if (refused !== undefined) {
      this.#say(refused, true);
      return;
    }

    const result = compareListings(left.listing.entries, right.listing.entries, foldsCase());
    left.marks.set(result.left);
    right.marks.set(result.right);
    left.select(result.select.left);
    right.select(result.select.right);
    this.#say(quickSummary(result.counts), false);
  }

  /** Opens the Synchronize dialog on the folders of the two panes. */
  synchronize() {
    const { left, right } = { left: this.#panes.left.view, right: this.#panes.right.view };
    const refused = refusal(left, right);
    const roots = { left: rootOf(left), right: rootOf(right) };

    if (refused !== undefined) {
      this.#say(refused, true);
    } else if (roots.left === undefined || roots.right === undefined) {
      this.#say('Wait until the folders have been listed.', true);
    } else {
      this.syncer.show({ left: roots.left, right: roots.right }, this.#hidden());
    }
  }
}
