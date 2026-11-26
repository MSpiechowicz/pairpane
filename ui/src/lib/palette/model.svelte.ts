import { type CommandId, COMMANDS, keysFor } from './commands';
import { score } from './fuzzy';

/** What the palette offers: a command, a folder to open in the active pane, or a name of the active folder to go to. `label` is
 * what is matched, shown through `displayName`. */
export type PaletteItem =
  | { kind: 'command'; id: CommandId; label: string; keys: string | undefined }
  | { kind: 'folder'; path: string; label: string }
  | { kind: 'file'; name: string; label: string };

/** Where the palette finds the folders and the names it offers. */
export interface PaletteSources {
  /** The folders visited last, the latest first. */
  folders: () => readonly string[];
  /** The names of the active folder. */
  names: () => readonly string[];
}

/** The most rows the palette shows for what is typed. */
export const MOST_SHOWN = 60;

/** How many recent folders show while nothing is typed. */
const RECENT_SHOWN = 8;

/** The order of the kinds, for items that score alike. */
const KIND_ORDER: Record<PaletteItem['kind'], number> = { command: 0, folder: 1, file: 2 };

/**
 * Ctrl+K: one field that finds a command, a recent folder or a name of the active folder by what is typed (its letters in
 * order, see `score`). With nothing typed it lists the commands and the latest folders. Up and Down move the choice,
 * Enter takes it.
 */
export class Palette {
  shown = $state(false);
  query = $state('');
  cursor = $state(0);

  readonly #sources: PaletteSources;

  constructor(sources: PaletteSources) {
    this.#sources = sources;
  }

  /** What matches the query, the best first, at most {@link MOST_SHOWN}: worked out once for each query. */
  get items(): PaletteItem[] {
    return this.#items;
  }

  readonly #items = $derived.by(() => this.#match(this.query));

  #match(query: string): PaletteItem[] {
    const commands: PaletteItem[] = COMMANDS.map(({ id, label }) => ({ kind: 'command', id, label, keys: keysFor(id) }));
    const folders: PaletteItem[] = this.#sources.folders().map((path) => ({ kind: 'folder', path, label: path }));

    // Every command, and the latest folders whatever their number.
    if (query.trim() === '') {
      return [...commands, ...folders.slice(0, RECENT_SHOWN)];
    }

    const names: PaletteItem[] = this.#sources.names().map((name) => ({ kind: 'file', name, label: name }));
    const ranked: { item: PaletteItem; score: number }[] = [];
    for (const item of [...commands, ...folders, ...names]) {
      const found = score(query, item.label);
      if (found !== undefined) {
        ranked.push({ item, score: found });
      }
    }

    ranked.sort((a, b) => b.score - a.score || KIND_ORDER[a.item.kind] - KIND_ORDER[b.item.kind]);

    return ranked.slice(0, MOST_SHOWN).map(({ item }) => item);
  }

  show() {
    this.query = '';
    this.cursor = 0;
    this.shown = true;
  }

  close() {
    this.shown = false;
  }

  /** The query changed: the best match is chosen again. */
  type(query: string) {
    this.query = query;
    this.cursor = 0;
  }

  /** Moves the choice by `delta`, round the ends. */
  move(delta: number) {
    const count = this.items.length;
    if (count > 0) {
      this.cursor = (this.cursor + delta + count) % count;
    }
  }

  /** Takes the item at `index` (the chosen one by default) and closes; `undefined` when there is none. */
  take(index = this.cursor): PaletteItem | undefined {
    const item = this.items[index];
    if (item !== undefined) {
      this.close();
    }

    return item;
  }
}
