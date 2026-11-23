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

/**
 * The names the pane `view` shows, in its order: those its filter lets through, loaded, at most {@link MOST_NAMES} (a
 * name the cursor could not go to is never offered).
 */
export function namesOf(view: PaneView): string[] {
  const names: string[] = [];

  for (let row = 0; row < view.count && names.length < MOST_NAMES; row += 1) {
    const entry = view.entryAt(row);
    if (entry !== undefined) {
      names.push(entry.name);
    }
  }

  return names;
}
