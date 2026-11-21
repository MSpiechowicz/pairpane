import { untrack } from 'svelte';

import type { SearchHit } from '$lib/ipc/bindings/SearchHit';

import type { ViewSession } from './session.svelte';

/** What the text and the hex view take. */
export interface ViewProps {
  session: ViewSession;
  /** The latest search match: the view goes to it and marks it. */
  hit?: SearchHit;
  /** Takes the keyboard when it is drawn (the overlay does; the quick view leaves it to the pane). */
  focus?: boolean;
}

/**
 * Calls `go` with the offset of each match that comes after the view is drawn; a match that was there before is
 * not a new one to go to. Call it while the component is made.
 */
export function onNewHit(hit: () => SearchHit | undefined, go: (offset: number) => void) {
  let seen = untrack(hit);

  $effect(() => {
    const current = hit();
    if (current !== undefined && current !== seen) {
      seen = current;
      untrack(() => go(current.offset));
    }
  });
}

/** An attachment that focuses the element when `focus` is set. */
export const takeFocus = (focus: boolean) => (node: HTMLElement) => {
  if (focus) {
    node.focus();
  }
};
