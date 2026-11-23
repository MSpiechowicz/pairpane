import type { PaneTabs } from '$lib/panes/tabs.svelte';
import type { PaneView } from '$lib/panes/view.svelte';

/** The most names of a folder the palette looks through. */
export const MOST_NAMES = 20_000;

/**
 * The folders the palette offers: what the panes have been to (the active pane's first, see `PaneTabs.recent`) and the
 * other pane's folder, the latest first, each once, the active pane's own folder left out.
 */
export function recentFolders([active, other]: readonly [PaneTabs, PaneTabs]): string[] {
  const here = active.view.listing.folder;
  const there = other.view.listing.folder;
  const found = [...(there === undefined ? [] : [there]), ...active.recent, ...other.recent];

  return [...new Set(found)].filter((path) => path !== here);
}

/** The names of the folder `view` lists (those loaded, at most {@link MOST_NAMES}). */
export function namesOf(view: PaneView): string[] {
  const names: string[] = [];

  for (const entry of view.listing.entries) {
    if (entry !== undefined) {
      names.push(entry.name);
      if (names.length >= MOST_NAMES) {
        break;
      }
    }
  }

  return names;
}
