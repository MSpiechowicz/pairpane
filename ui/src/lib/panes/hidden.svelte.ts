import { untrack } from 'svelte';

import type { PaneId } from '$lib/ipc/bindings/PaneId';
import { type Preferences, preferences } from '$lib/shell/preferences.svelte';

import { shows } from './columns';
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
  // a sort by a column that is not on the screen could not be seen or changed.
  $effect(() => {
    for (const tabs of [panes.left, panes.right]) {
      const view = tabs.view;
      if (!shows(view.column, prefs.value)) {
        untrack(() => view.restoreSort('name', false));
      }
    }
  });
}
