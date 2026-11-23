import { untrack } from 'svelte';

import type { PaneId } from '$lib/ipc/bindings/PaneId';
import { type Preferences, preferences } from '$lib/shell/preferences.svelte';

import { shows } from './columns';
import type { SortColumn } from './order';
import type { PaneTabs } from './tabs.svelte';

/**
 * Makes both panes follow the preferences. Lists both panes again whenever the preference for hidden files changes (the Hidden button, the settings, a
 * restored session). A folder opened after the change is listed as it says anyway (see `ViewHooks.showHidden`), so
 * only the folders already shown are listed again. Call while a component is being set up: it starts an effect.
 */
export function followPreferences(panes: Record<PaneId, PaneTabs>, prefs: Preferences = preferences) {
  $effect(() => {
    const show = prefs.value.showHidden;

    untrack(() => {
      for (const tabs of [panes.left, panes.right]) {
        void tabs.view.listing.setShowHidden(show);
      }
    });
  });

  // A pane sorted by a column the settings hide (now, or a tab that brings its sort back) is sorted by name instead:
  // a sort by a column that is not on the screen could not be seen or changed. The sort it had comes back when the
  // column is shown again, unless the pane was sorted otherwise meanwhile.
  const displaced = new Map<PaneId, { column: SortColumn; desc: boolean }>();
  $effect(() => {
    for (const side of ['left', 'right'] as const) {
      const view = panes[side].view;
      const column = view.column;
      const desc = view.desc;
      const before = displaced.get(side);

      if (!shows(column, prefs.value)) {
        displaced.set(side, { column, desc });
        untrack(() => view.restoreSort('name', false));
      } else if (before !== undefined && (column !== 'name' || desc)) {
        displaced.delete(side); // sorted otherwise meanwhile
      } else if (before !== undefined && shows(before.column, prefs.value)) {
        displaced.delete(side);
        untrack(() => view.restoreSort(before.column, before.desc));
      }
    }
  });
}
