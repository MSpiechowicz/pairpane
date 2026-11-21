import { tick } from 'svelte';

import type { PaneId } from '$lib/ipc/bindings/PaneId';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { focusActiveList } from '$lib/panes/focus';
import { InlineRename } from '$lib/panes/inlineRename.svelte';
import type { PaneTabs } from '$lib/panes/tabs.svelte';
import type { PaneView } from '$lib/panes/view.svelte';

import { type FileAction, type Notice, trashName, what } from './dialogs';
import { type DialogKind, type OpRequest, requestFor, targets } from './targets';

/** The copy, move, rename, new item or link dialog that is open, and the panes it works between. */
export interface OpenDialog {
  request: OpRequest;
  source: PaneView;
  other: PaneView;
}

/** The confirmation to move items to the Trash or to delete them for good. */
export interface Removal {
  permanent: boolean;
  path: string;
  names: string[];
  source: PaneView;
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

  readonly #ipc: Transport;
  readonly #panes: Record<PaneId, PaneTabs>;
  readonly #active: () => PaneId;
  #toasts = 0;

  constructor(ipc: Transport, panes: Record<PaneId, PaneTabs>, active: () => PaneId) {
    this.#ipc = ipc;
    this.#panes = panes;
    this.#active = active;
    this.rename = new InlineRename(ipc, (text) => this.say(text, true));
  }

  /** A dialog has the keyboard: the shortcuts and Tab are its own then. */
  get modal(): boolean {
    return this.dialog !== undefined || this.removal !== undefined;
  }

  /** Does what the key or menu item asks, for the active pane; nothing when there is nothing to act on. */
  run(action: FileAction) {
    if (action === 'renameInline') {
      const tabs = this.#panes[this.#active()];
      this.rename.begin(tabs.view, tabs.active);
    } else if (action === 'trash' || action === 'deletePermanently') {
      this.#ask(action === 'deletePermanently');
    } else {
      this.#open(DIALOGS[action] as DialogKind);
    }

    this.#settle();
  }

  #open(kind: DialogKind) {
    const active = this.#active();
    const source = this.#panes[active].view;
    const other = this.#panes[active === 'left' ? 'right' : 'left'].view;
    const request = requestFor(kind, source, other);
    if (request !== undefined) {
      this.dialog = { request, source, other };
    }
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
      if (request.kind === 'copy' || request.kind === 'move') {
        source.deselect(request.names);
      } else if (request.kind === 'symlink' || request.kind === 'hardlink') {
        other.pointAt(name);
      } else {
        source.pointAt(name);
      }
    }

    this.#settle();
  }

  #ask(permanent: boolean) {
    const source = this.#panes[this.#active()].view;
    const path = source.listing.path;
    const names = targets(source);
    if (path !== undefined && names.length > 0) {
      this.removal = { permanent, path, names, source };
    }
  }

  closeRemoval() {
    this.removal = undefined;
    this.#settle();
  }

  /** The confirmed removal; a refusal rejects, so that the dialog can show it and stay. */
  async confirmRemoval() {
    const removal = this.removal;
    if (removal === undefined) {
      return;
    }

    const { path, names, source } = removal;
    if (removal.permanent) {
      await this.#ipc.invoke('ops_delete', { path, names });
      source.deselect(names);
    } else {
      const { undo, failure } = await this.#ipc.invoke('ops_trash', { path, names });
      this.#trashed(names, undo, failure?.message);
      if (failure === null) {
        source.deselect(names);
      }
    }

    this.closeRemoval();
  }

  /** Says what the move did; a move that stopped part way still offers to take back the items that went. */
  #trashed(names: string[], undo: number | null, failure: string | undefined) {
    const place = `the ${trashName()}`;
    if (failure === undefined) {
      this.say(`Moved ${what(names)} to ${place}.`, false, undo);
    } else if (undo === null) {
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
