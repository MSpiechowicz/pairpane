import { tick } from 'svelte';

import { deselectResults, groupResults, type ResultGroup, type Within } from '$lib/find/results';
import type { PaneId } from '$lib/ipc/bindings/PaneId';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { archiveName } from '$lib/panes/archive';
import { focusActiveList } from '$lib/panes/focus';
import { InlineRename } from '$lib/panes/inlineRename.svelte';
import type { PaneTabs } from '$lib/panes/tabs.svelte';
import type { PaneView } from '$lib/panes/view.svelte';

import { type FileAction, type Notice, trashName, what } from './dialogs';
import { chosen, type DialogKind, type OpRequest, READ_ONLY_NOTICE, refusal, requestFor, resultTargets, targets } from './targets';
import { isTool, ItemTools } from './tools.svelte';

/** The copy, move, add to archive, extract, combine, rename, new item or link dialog that is open, and the panes it works between. */
export interface OpenDialog {
  request: OpRequest;
  source: PaneView;
  other: PaneView;
}

/**
 * The confirmation to move items to the Trash or to delete them for good. `results`: the items are search results,
 * `path` is the folder searched and the names are paths below it.
 */
export interface Removal {
  permanent: boolean;
  path: string;
  names: string[];
  source: PaneView;
  results?: true;
}

/** The card that says what happened; `undo` is the token that takes a move to the Trash back. */
export interface ToastState extends Notice {
  id: number;
  undo: number | null;
}

/** The dialog each action opens. */
const DIALOGS: Partial<Record<FileAction, DialogKind>> = {
  copy: 'copy',
  move: 'move',
  rename: 'rename',
  newFolder: 'mkdir',
  newFile: 'mkfile',
  symlink: 'symlink',
  hardlink: 'hardlink',
  combine: 'combine',
  extract: 'extract',
};

/**
 * What the file keys and the Files menu do on the active pane: the dialogs, the Trash with its undo, renaming in
 * place, and the card that reports. The window shows `dialog`, `removal` and `toast` and hands the user's answers
 * back; the actions themselves live here.
 */
export class FileActions {
  dialog = $state.raw<OpenDialog | undefined>();
  removal = $state.raw<Removal | undefined>();
  toast = $state.raw<ToastState | undefined>();
  readonly rename: InlineRename;
  readonly tools: ItemTools;

  readonly #ipc: Transport;
  readonly #panes: Record<PaneId, PaneTabs>;
  readonly #active: () => PaneId;
  #toasts = 0;

  constructor(ipc: Transport, panes: Record<PaneId, PaneTabs>, active: () => PaneId) {
    this.#ipc = ipc;
    this.#panes = panes;
    this.#active = active;
    this.rename = new InlineRename(ipc, (text) => this.say(text, true));
    this.tools = new ItemTools(ipc, (text, error) => this.say(text, error));
  }

  /** A dialog has the keyboard: the shortcuts and Tab are its own then. */
  get modal(): boolean {
    return this.dialog !== undefined || this.removal !== undefined || this.tools.modal;
  }

  /**
   * Does what the key or menu item asks, for the active pane; nothing when there is nothing to act on. An action
   * that would change an archive, or write into a pane that shows one, is refused in a card instead.
   */
  run(action: FileAction) {
    const active = this.#active();
    const source = this.#panes[active].view;
    const other = this.#panes[active === 'left' ? 'right' : 'left'].view;
    const refused = refusal(action, source, other);

    if (refused !== undefined) {
      this.say(refused, true);
    } else if (action === 'renameInline') {
      const tabs = this.#panes[active];
      this.rename.begin(tabs.view, tabs.active);
    } else if (action === 'trash' || action === 'deletePermanently') {
      this.#ask(action === 'deletePermanently');
    } else if (isTool(action)) {
      this.tools.run(action, source, other);
    } else {
      this.#open(DIALOGS[action] as DialogKind, source, other);
    }

    this.#settle();
  }

  /** F4 on an archive's entry: says that the archive is read-only, and whether it did. */
  refuseEdit(): boolean {
    if (!this.#panes[this.#active()].view.listing.readonly) {
      return false;
    }

    this.say(READ_ONLY_NOTICE, true);
    return true;
  }

  #open(kind: DialogKind, source: PaneView, other: PaneView) {
    const request = requestFor(kind, source, other);
    if (request !== undefined) {
      this.dialog = { request, source, other };
    } else if (kind === 'combine' && source.entryAt(source.cursor) !== undefined) {
      this.say('Choose the first part of a split file, named like “video.mkv.001”.', true);
    } else if (kind === 'extract' && chosen(source).length > 0) {
      this.say('Choose an archive to extract.', true);
    }
  }

  /** The tool's dialog closed without doing anything. */
  closeTool() {
    this.tools.close();
    this.#settle();
  }

  /** A tool's job started. */
  toolDone(output?: string) {
    this.tools.done(output);
    this.#settle();
  }

  /** The dialog closed without doing anything. */
  closeDialog() {
    this.dialog = undefined;
    this.#settle();
  }

  /** A job started, an item was renamed, made or linked: the cursor goes to it (a started job took the selection). */
  dialogDone(name: string) {
    const dialog = this.dialog;
    this.dialog = undefined;

    if (dialog !== undefined) {
      const { request, source, other } = dialog;
      if (request.kind === 'copy' && request.results) {
        deselectResults(source, request.names);
      } else if (request.kind === 'copy' || request.kind === 'move' || request.kind === 'add') {
        source.deselect(request.names);
      } else if (request.kind === 'extract') {
        source.deselect(request.names.length > 0 ? request.names : request.archives.map(({ archive }) => archiveName(archive)));
      } else if (request.kind === 'symlink' || request.kind === 'hardlink') {
        other.pointAt(name);
      } else if (request.kind !== 'combine') {
        source.pointAt(name);
      }
    }

    this.#settle();
  }

  #ask(permanent: boolean) {
    const source = this.#panes[this.#active()].view;
    const { path, folder, results } = source.listing;

    if (results !== undefined) {
      const names = resultTargets(source);
      if (folder !== undefined && names.length > 0) {
        this.removal = { permanent, path: folder, names, source, results: true };
      }

      return;
    }

    const names = targets(source);
    if (path !== undefined && names.length > 0) {
      this.removal = { permanent, path, names, source };
    }
  }

  closeRemoval() {
    this.removal = undefined;
    this.#settle();
  }

  /**
   * The confirmed removal; a refusal rejects, so that the dialog can show it and stay. Search results are in many
   * folders: one request for each, and the results are listed again after.
   */
  async confirmRemoval() {
    const removal = this.removal;
    if (removal === undefined) {
      return;
    }

    const { path, names } = removal;
    const groups: ResultGroup[] = removal.results ? groupResults(path, names) : [{ folder: path, dir: '', names, rels: names }];
    const within = (dir: string): Within | undefined => (removal.results ? { root: path, dir } : undefined);
    const gone: string[] = [];

    try {
      if (removal.permanent) {
        await this.#delete(groups, within, gone);
      } else {
        await this.#trash(groups, within, gone);
      }
    } finally {
      this.#removed(removal, gone);
    }

    this.closeRemoval();
  }

  /** Deletes the groups in turn; `gone` collects the items that are. A refusal rejects. */
  async #delete(groups: ResultGroup[], within: (dir: string) => Within | undefined, gone: string[]) {
    for (const { folder, dir, names, rels } of groups) {
      await this.#ipc.invoke('ops_delete', { path: folder, names, within: within(dir) });
      gone.push(...rels);
    }
  }

  /**
   * Moves the groups to the Trash in turn, one request (and undo token) for each, and says what happened; `gone`
   * collects the items that went. The move stops at the first group that does not go through whole; a refusal of
   * the first rejects, a later one is told with what already moved. `within` is what search results ask the backend
   * to check of their folder (`undefined` for a plain folder).
   */
  async #trash(groups: ResultGroup[], within: (dir: string) => Within | undefined, gone: string[]) {
    const undos: number[] = [];
    let failure: string | undefined;

    try {
      for (const { folder, dir, names, rels } of groups) {
        const result = await this.#ipc.invoke('ops_trash', { path: folder, names, within: within(dir) });
        if (result.undo !== null) {
          undos.push(result.undo);
        }

        if (result.failure !== null) {
          failure = result.failure.message;
          break;
        }

        gone.push(...rels);
      }
    } catch (error) {
      if (gone.length === 0 && undos.length === 0) {
        throw error;
      }

      failure = IpcFailure.from(error).message;
    }

    this.#trashed(gone, undos, failure);
  }

  /** The removed items are not selected any more; search results are listed again, and a dialog that stays asks only about the rest. */
  #removed(removal: Removal, gone: readonly string[]) {
    if (!removal.results) {
      removal.source.deselect(gone);
      return;
    }

    deselectResults(removal.source, gone);
    void removal.source.listing.reload();
    if (this.removal === removal && gone.length > 0) {
      const removed = new Set(gone);
      this.removal = { ...removal, names: removal.names.filter((name) => !removed.has(name)) };
    }
  }

  /**
   * Says what the move did; a move that stopped part way still offers to take back the items that went. One undo
   * token takes back one request, so Undo is offered only when the move was a single one (one folder).
   */
  #trashed(moved: string[], undos: number[], failure: string | undefined) {
    const place = `the ${trashName()}`;
    const undo = undos.length === 1 ? undos[0] : null;
    if (failure === undefined) {
      this.say(`Moved ${what(moved)} to ${place}.`, false, undo);
    } else if (moved.length === 0 && undos.length === 0) {
      this.say(failure, true);
    } else {
      this.say(`Some items were moved to ${place}, but not all. ${failure}`, true, undo);
    }
  }

  /** Puts the move to the Trash the card tells of back. */
  async undo() {
    const shown = this.toast;
    const token = shown?.undo;
    if (shown === undefined || token === null || token === undefined) {
      return;
    }

    this.toast = { ...shown, undo: null }; // a second click has nothing to undo

    try {
      const count = await this.#ipc.invoke('ops_untrash', { token });
      this.say(`Restored ${count === 1 ? '1 item' : `${count} items`}.`, false);
    } catch (error) {
      const failure = IpcFailure.from(error);
      // The backend keeps the record when something is in the way, the restore failed or it was busy: Undo again.
      const again = failure.code === 'invalidInput' || failure.code === 'io' || failure.code === 'busy';

      this.say(failure.message, true, again ? token : null);
    }

    this.#settle();
  }

  say(text: string, error: boolean, undo: number | null = null) {
    this.#toasts += 1;
    this.toast = { id: this.#toasts, text, error, undo };
  }

  dismissToast() {
    this.toast = undefined;
    this.#settle();
  }

  /** A dialog, a menu item or a card that had the focus is gone: the list takes it back. */
  #settle() {
    void tick().then(focusActiveList);
  }
}
