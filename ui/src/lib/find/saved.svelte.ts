import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';
import type { SavedSearch } from '$lib/ipc/bindings/SavedSearch';

/** Searches kept (the backend keeps this many and drops the rest). */
export const MAX_SEARCHES = 50;
/** The longest name of a saved search, in characters. */
export const MAX_SEARCH_NAME = 64;

/** The searches kept by name: their criteria only, the folder is whichever pane is in when one is loaded. */
export class SavedSearches {
  /** Replaced on every change, never changed in place. */
  #list = $state.raw<SavedSearch[]>([]);

  get list(): readonly SavedSearch[] {
    return this.#list;
  }

  /**
   * Keeps `criteria` under `name` (trimmed; the same name in any case is the same search and is replaced). Returns
   * why it was not kept, as the backend would refuse it.
   */
  save(name: string, criteria: FindCriteria): string | undefined {
    const trimmed = name.trim();
    if (trimmed === '') {
      return 'Type a name for the search.';
    }

    if ([...trimmed].length > MAX_SEARCH_NAME) {
      return `A name has at most ${MAX_SEARCH_NAME} characters.`;
    }

    const lower = trimmed.toLowerCase();
    const at = this.#list.findIndex((search) => search.name.toLowerCase() === lower);
    if (at < 0 && this.#list.length >= MAX_SEARCHES) {
      return `Up to ${MAX_SEARCHES} searches can be kept: delete one first.`;
    }

    const search = { name: trimmed, criteria };
    this.#list = at < 0 ? [...this.#list, search] : this.#list.map((kept, index) => (index === at ? search : kept));
    return undefined;
  }

  find(name: string): SavedSearch | undefined {
    return this.#list.find((search) => search.name === name);
  }

  remove(name: string) {
    this.#list = this.#list.filter((search) => search.name !== name);
  }

  /** Takes over the searches of an earlier session. */
  restore(searches: readonly SavedSearch[]) {
    this.#list = searches.slice(0, MAX_SEARCHES);
  }
}
