import type { Entry } from '$lib/ipc/bindings/Entry';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import type { PaneView } from '$lib/panes/view.svelte';

import { chosen } from './targets';

/** The file tools: what the Files menu and Alt+Enter ask for besides copying, moving and the like. */
export type ToolAction = 'properties' | 'attributes' | 'checksum' | 'verifyChecksum' | 'split' | 'pack';

/** The dialog of a tool that is open; `source` is the pane its items are in. */
export type ToolDialog =
  | { kind: 'properties'; path: string; entry: Entry; source: PaneView }
  | { kind: 'attributes'; path: string; entries: Entry[]; source: PaneView }
  | { kind: 'checksum'; path: string; names: string[]; source: PaneView }
  /** The parts go into the folder `to` to begin with: the other pane's. */
  | { kind: 'split'; path: string; entry: Entry; to: string; source: PaneView }
  /** The archive is made in the folder `to` to begin with: the other pane's (the folder of the items when that pane is in an archive). */
  | { kind: 'pack'; path: string; names: string[]; to: string; source: PaneView };

const TOOLS: ReadonlySet<string> = new Set<ToolAction>(['properties', 'attributes', 'checksum', 'verifyChecksum', 'split', 'pack']);

export const isTool = (action: string): action is ToolAction => TOOLS.has(action);

/** What a tool says when the item under the cursor is not one it works on. */
const NOT_A_FILE = {
  verifyChecksum: 'Choose a checksum file to verify.',
  split: 'Choose a file to split.',
} as const;

/**
 * The properties dialog, the attributes dialog, the checksum file, the split, the archive and the check of a checksum
 * file, on the items of the active pane. The window shows `dialog` and hands the user's answers back; `say` is how a
 * refusal that needs no dialog reaches the user.
 */
export class ItemTools {
  dialog = $state.raw<ToolDialog | undefined>();

  readonly #ipc: Transport;
  readonly #say: (text: string, error: boolean) => void;

  constructor(ipc: Transport, say: (text: string, error: boolean) => void) {
    this.#ipc = ipc;
    this.#say = say;
  }

  /** A dialog has the keyboard: the shortcuts and Tab are its own then. */
  get modal(): boolean {
    return this.dialog !== undefined;
  }

  /**
   * Does what the tool asks on `source`'s items; nothing when there is nothing to act on. Properties, splitting and
   * checking act on the entry under the cursor (the others on the selection, as copying does); the parts of a split
   * and the archive go to the other pane's folder.
   */
  run(action: ToolAction, source: PaneView, other: PaneView) {
    const path = source.listing.path;
    const under = source.entryAt(source.cursor);
    const entries = chosen(source);
    if (path === undefined || entries.length === 0) {
      return;
    }

    switch (action) {
      case 'attributes':
        this.dialog = { kind: 'attributes', path, entries, source };
        break;
      case 'checksum':
        this.dialog = { kind: 'checksum', path, names: entries.map((entry) => entry.name), source };
        break;
      case 'pack':
        this.dialog = { kind: 'pack', path, names: entries.map((entry) => entry.name), to: other.listing.path ?? path, source };
        break;
      case 'properties':
        if (under !== undefined) {
          this.dialog = { kind: 'properties', path, entry: under, source };
        }
        break;
      default:
        if (under !== undefined) {
          this.#oneFile(action, path, under, source, other);
        }
    }
  }

  #oneFile(action: 'verifyChecksum' | 'split', path: string, entry: Entry, source: PaneView, other: PaneView) {
    if (entry.kind !== 'file') {
      this.#say(NOT_A_FILE[action], true);
    } else if (action === 'split') {
      this.dialog = { kind: 'split', path, entry, to: other.listing.path ?? path, source };
    } else {
      void this.#verify(path, entry.name);
    }
  }

  /** Starts the check of a checksum file; the job shows in the Transfers panel, a refusal in the card. */
  async #verify(path: string, name: string) {
    try {
      await this.#ipc.invoke('ops_task', { from: path, names: [name], task: { kind: 'verifyChecksum' } });
    } catch (error) {
      this.#say(IpcFailure.from(error).message, true);
    }
  }

  /** The dialog closed without doing anything. */
  close() {
    this.dialog = undefined;
  }

  /** A job started: a checksum file or an archive takes the selection it was made for, and the cursor goes to a new checksum file. */
  done(output?: string) {
    const dialog = this.dialog;
    this.dialog = undefined;

    if (dialog?.kind === 'checksum' && output !== undefined) {
      dialog.source.deselect(dialog.names);
      dialog.source.pointAt(output);
    } else if (dialog?.kind === 'pack') {
      dialog.source.deselect(dialog.names);
    }
  }
}
