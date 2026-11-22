import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';
import type { SavedSearch } from '$lib/ipc/bindings/SavedSearch';
import { MAX_SAVED, MAX_SAVED_NAME, SavedNamed } from '$lib/saved/named.svelte';

/** Searches kept (the backend keeps this many and drops the rest). */
export const MAX_SEARCHES = MAX_SAVED;
/** The longest name of a saved search, in characters. */
export const MAX_SEARCH_NAME = MAX_SAVED_NAME;

/** The searches kept by name: their criteria only, the folder is whichever pane is in when one is loaded. */
export class SavedSearches extends SavedNamed<SavedSearch, FindCriteria> {
  constructor() {
    super(
      { one: 'search', many: 'searches' },
      (name, criteria) => ({ name, criteria }),
      (search) => search.criteria,
    );
  }
}
