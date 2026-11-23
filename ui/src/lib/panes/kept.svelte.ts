import { untrack } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';

/** A cache of a bounded size: when it is full the least recently wanted value goes. Reactive, so a cell shows its value on arrival. */
export class Kept<T> {
  readonly #values = new SvelteMap<string, T>();
  /** The keys of `#values`, the least recently wanted first. */
  readonly #recent = new Map<string, true>();
  readonly #max: number;

  constructor(max: number) {
    this.#max = max;
  }

  get(key: string): T | undefined {
    return this.#values.get(key);
  }

  /** The value of `key` without making the reader depend on it (a pane's effect asks about what it has before it asks for more). */
  peek(key: string): T | undefined {
    return untrack(() => this.#values.get(key));
  }

  /** Whether `key` is kept; if it is, it is now the most recently wanted. Not reactive. */
  wanted(key: string): boolean {
    if (!this.#recent.has(key)) {
      return false;
    }

    this.#touch(key);
    return true;
  }

  set(key: string, value: T) {
    this.#values.set(key, value);
    this.#touch(key);

    for (const oldest of this.#recent.keys()) {
      if (this.#recent.size <= this.#max) {
        break;
      }

      this.#recent.delete(oldest);
      this.#values.delete(oldest);
    }
  }

  #touch(key: string) {
    this.#recent.delete(key);
    this.#recent.set(key, true);
  }
}
