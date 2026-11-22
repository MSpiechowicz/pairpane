import type { Entry } from '$lib/ipc/bindings/Entry';

/**
 * The names a pane has selected. Kept by name, so the selection follows the entries through re-sorting and reloads.
 * A plain set (not proxied); readers track `#revision`, bumped on every change.
 */
export class Selection {
  readonly #names = new Set<string>();
  #revision = $state(0);

  has(name: string): boolean {
    void this.#revision;
    return this.#names.has(name);
  }

  get size(): number {
    void this.#revision;
    return this.#names.size;
  }

  toggle(name: string) {
    if (!this.#names.delete(name)) {
      this.#names.add(name);
    }

    this.#revision += 1;
  }

  add(names: Iterable<string>) {
    for (const name of names) {
      this.#names.add(name);
    }

    this.#revision += 1;
  }

  /** Stops selecting `names`; how many of them were selected. */
  remove(names: Iterable<string>): number {
    let removed = 0;
    for (const name of names) {
      if (this.#names.delete(name)) {
        removed += 1;
      }
    }

    if (removed > 0) {
      this.#revision += 1;
    }

    return removed;
  }

  /** Keeps only the names that are `present`. */
  retain(present: ReadonlySet<string>) {
    this.remove([...this.#names].filter((name) => !present.has(name)));
  }

  clear() {
    this.#names.clear();
    this.#revision += 1;
  }
}

/** What `entries` add up to: folders and files counted, sizes summed (a folder's is its computed size, if any). */
export function summarize(
  entries: readonly Entry[],
  folderSize: (name: string) => number | undefined,
): { dirs: number; files: number; bytes: number } {
  let dirs = 0;
  let files = 0;
  let bytes = 0;

  for (const entry of entries) {
    if (entry.kind === 'dir') {
      dirs += 1;
      bytes += folderSize(entry.name) ?? 0;
    } else {
      files += 1;
      bytes += entry.size;
    }
  }

  return { dirs, files, bytes };
}
