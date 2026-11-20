/** Favourite folders kept. */
export const MAX_HOTLIST = 100;

/** The favourite folders, as paths in the order they were added. */
export class Hotlist {
  /** Replaced on every change, never changed in place. */
  #paths = $state.raw<string[]>([]);

  get paths(): readonly string[] {
    return this.#paths;
  }

  has(path: string): boolean {
    return this.#paths.includes(path);
  }

  /** Adds a folder at the end. A folder that is there already, or a full list, changes nothing. */
  add(path: string): boolean {
    if (this.has(path) || this.#paths.length >= MAX_HOTLIST) {
      return false;
    }

    this.#paths = [...this.#paths, path];
    return true;
  }

  remove(path: string) {
    this.#paths = this.#paths.filter((candidate) => candidate !== path);
  }

  /** The favourites whose path contains `text`, ignoring case; all of them for an empty text. */
  filter(text: string): string[] {
    const needle = text.toLowerCase();
    return this.#paths.filter((path) => path.toLowerCase().includes(needle));
  }

  /** Takes over the favourites of an earlier session. */
  restore(paths: readonly string[]) {
    this.#paths = [...new Set(paths)].slice(0, MAX_HOTLIST);
  }
}
