/** Named items kept (the backend keeps this many and drops the rest). */
export const MAX_SAVED = 50;
/** The longest name of a saved item, in characters. */
export const MAX_SAVED_NAME = 64;

/** What the items are called in the messages and the labels of the bar: `search`, `searches`. */
export interface Words {
  one: string;
  many: string;
}

/**
 * Items kept by name, in the order they were saved: a saved search, a saved rename rule. `T` is the item the
 * backend keeps and `V` what it holds besides the name; `make` and `valueOf` convert between the two.
 */
export class SavedNamed<T extends { name: string }, V> {
  /** Replaced on every change, never changed in place. */
  #list = $state.raw<T[]>([]);

  readonly words: Words;
  readonly #make: (name: string, value: V) => T;
  readonly #value: (item: T) => V;

  constructor(words: Words, make: (name: string, value: V) => T, value: (item: T) => V) {
    this.words = words;
    this.#make = make;
    this.#value = value;
  }

  get list(): readonly T[] {
    return this.#list;
  }

  /**
   * Keeps `value` under `name` (trimmed; the same name in any case is the same item and is replaced). Returns why it
   * was not kept, as the backend would refuse it.
   */
  save(name: string, value: V): string | undefined {
    const trimmed = name.trim();
    if (trimmed === '') {
      return `Type a name for the ${this.words.one}.`;
    }

    if ([...trimmed].length > MAX_SAVED_NAME) {
      return `A name has at most ${MAX_SAVED_NAME} characters.`;
    }

    const lower = trimmed.toLowerCase();
    const at = this.#list.findIndex((item) => item.name.toLowerCase() === lower);
    if (at < 0 && this.#list.length >= MAX_SAVED) {
      return `Up to ${MAX_SAVED} ${this.words.many} can be kept: delete one first.`;
    }

    const item = this.#make(trimmed, value);
    this.#list = at < 0 ? [...this.#list, item] : this.#list.map((kept, index) => (index === at ? item : kept));
    return undefined;
  }

  find(name: string): T | undefined {
    return this.#list.find((item) => item.name === name);
  }

  /** What the item `name` holds, to load into a form. */
  valueFor(name: string): V | undefined {
    const item = this.find(name);
    return item === undefined ? undefined : this.#value(item);
  }

  remove(name: string) {
    this.#list = this.#list.filter((item) => item.name !== name);
  }

  /** Takes over the items of an earlier session. */
  restore(items: readonly T[]) {
    this.#list = items.slice(0, MAX_SAVED);
  }
}
