import { tick } from 'svelte';

import { deselectResults, groupResults, type ResultGroup, type Within } from '$lib/find/results';
import type { JobId } from '$lib/ipc/bindings/JobId';
import type { PaneId } from '$lib/ipc/bindings/PaneId';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { archiveName } from '$lib/panes/archive';
import { focusActiveList } from '$lib/panes/focus';
import { InlineRename } from '$lib/panes/inlineRename.svelte';
import type { PaneTabs } from '$lib/panes/tabs.svelte';
import type { PaneView } from '$lib/panes/view.svelte';
import { SyncReports } from '$lib/compare/undo';
import { RenameReports } from '$lib/rename/undo';
import { type Preferences, preferences } from '$lib/shell/preferences.svelte';

import { type FileAction, type Notice, perform, specOf, trashName, what } from './dialogs';
import {
  chosen,
  type DialogKind,
  type OpRequest,
  READ_ONLY_NOTICE,
  refusal,
  REMOTE_NOTICE,
  RESULTS_NOTICE,
  requestFor,
  resultTargets,
  targets,
} from './targets';
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

/** Added to the card about a move to the Trash that took several requests (search results in several folders). */
const NO_UNDO_NOTICE = 'Undo is only offered when one folder is affected.';

/** What the Undo of a card takes back: a move to the Trash (by its token) or a rename (by its job). */
export type ToastUndo = { kind: 'trash'; token: number } | { kind: 'rename'; job: JobId } | { kind: 'sync'; job: JobId };

/** The card that says what happened; `undo` is what its Undo button takes back. */
export interface ToastState extends Notice {
  id: number;
  undo: ToastUndo | null;
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
  /** Tells how a rename and its undo went, in a card. */
  readonly renames = new RenameReports(({ text, error, undo }) => this.say(text, error, undo === null ? null : { kind: 'rename', job: undo }));
  readonly syncs = new SyncReports(({ text, error, undo }) => this.say(text, error, undo === null ? null : { kind: 'sync', job: undo }));

  readonly #ipc: Transport;
  readonly #panes: Record<PaneId, PaneTabs>;
  readonly #active: () => PaneId;
  readonly #preferences: Preferences;
  #toasts = 0;
  /** A copy, move or removal that skipped its question is being asked of the backend: a second key press waits for it. */
  #direct = false;

  constructor(ipc: Transport, panes: Record<PaneId, PaneTabs>, active: () => PaneId, prefs: Preferences = preferences) {
    this.#ipc = ipc;
    this.#panes = panes;
    this.#active = active;
    this.#preferences = prefs;
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

  /**
   * F4 on an archive's entry (read-only), on a file on a server or on a search result (its folder has to be opened
   * first, so that the edit is of the file in the folder that is there): says why, and whether it did.
   */
  refuseEdit(): boolean {
    const { readonly, results, remote } = this.#panes[this.#active()].view.listing;

    if (remote !== undefined) {
      this.say(REMOTE_NOTICE, true);
      return true;
    }

    if (results !== undefined) {
      this.say(RESULTS_NOTICE, true);
      return true;
    }

    if (readonly) {
      this.say(READ_ONLY_NOTICE, true);
      return true;
    }

    return false;
  }

  #open(kind: DialogKind, source: PaneView, other: PaneView) {
    const request = requestFor(kind, source, other);
    if (request !== undefined && this.#skipsPrompt(kind, request)) {
      void this.#goStraight({ request, source, other });
    } else if (request !== undefined) {
      this.dialog = { request, source, other };
    } else if (kind === 'combine' && source.entryAt(source.cursor) !== undefined) {
      this.say('Choose the first part of a split file, named like “video.mkv.001”.', true);
    } else if (kind === 'extract' && chosen(source).length > 0) {
      this.say('Choose an archive to extract.', true);
    }
  }

  /**
   * Whether F5 and F6 go to the other pane's folder without asking (the setting): only when there is a folder to go to.
   * A copy into an archive and an extraction by F5 go straight too; the other dialogs ask for something of their own.
   */
  #skipsPrompt(kind: DialogKind, request: OpRequest): boolean {
    if (this.#preferences.value.confirmTransfer || (kind !== 'copy' && kind !== 'move')) {
      return false;
    }

    return request.kind === 'add' || request.kind === 'remote' || ('to' in request && request.to !== '');
  }

  /** Does what the dialog would do with the target it starts with; a refusal is told in a card, as the dialog would show it. */
  async #goStraight(dialog: OpenDialog) {
    if (this.#direct) {
      return;
    }

    this.#direct = true;

    try {
      const done = await perform(this.#ipc, dialog.request, specOf(dialog.request).initial, false);
      this.#finish(dialog, done.name);
    } catch (error) {
      this.say(IpcFailure.from(error).message, true);
    } finally {
      this.#direct = false;
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
      this.#finish(dialog, name);
    } else {
      this.#settle();
    }
  }

  /** `dialog`'s work is done (it started a job, or made or changed an item named `name`): the selection and the cursor follow. */
  #finish({ request, source, other }: OpenDialog, name: string) {
    if (request.kind === 'copy' && request.results) {
      deselectResults(source, request.names);
    } else if (request.kind === 'copy' || request.kind === 'move' || request.kind === 'add' || request.kind === 'remote') {
      source.deselect(request.names);
    } else if (request.kind === 'extract') {
      source.deselect(request.names.length > 0 ? request.names : request.archives.map(({ archive }) => archiveName(archive)));
    } else if (request.kind === 'symlink' || request.kind === 'hardlink') {
      other.pointAt(name);
    } else if (request.kind !== 'combine') {
      source.pointAt(name);
    }

    this.#settle();
  }

  /** The Delete key: asks first, but for a move to the Trash when the setting says not to (Undo is in the card that tells it). */
  #ask(permanent: boolean) {
    const removal = this.#toRemove(permanent);
    if (removal === undefined) {
      return;
    }

    if (permanent || this.#preferences.value.confirmTrash) {
      this.removal = removal;
    } else {
      void this.#trashStraight(removal);
    }
  }

  /** What the active pane's selection (or cursor) would remove; `undefined` when there is nothing to remove. */
  #toRemove(permanent: boolean): Removal | undefined {
    const source = this.#panes[this.#active()].view;
    const { path, folder, results } = source.listing;

    if (results !== undefined) {
      const names = resultTargets(source);
      return folder !== undefined && names.length > 0 ? { permanent, path: folder, names, source, results: true } : undefined;
    }

    const names = targets(source);
    return path !== undefined && names.length > 0 ? { permanent, path, names, source } : undefined;
  }

  /** Moves to the Trash without the question; a refusal is told in a card (the dialog would have shown it). */
  async #trashStraight(removal: Removal) {
    if (this.#direct) {
      return;
    }

    this.#direct = true;

    try {
      await this.#remove(removal);
    } catch (error) {
      this.say(IpcFailure.from(error).message, true);
    } finally {
      this.#direct = false;
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

    await this.#remove(removal);
    this.closeRemoval();
  }

  /** Removes what `removal` names (see {@link confirmRemoval}). */
  async #remove(removal: Removal) {
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
    const undo: ToastUndo | null = undos.length === 1 ? { kind: 'trash', token: undos.at(0) as number } : null;
    const why = undos.length > 1 ? ` ${NO_UNDO_NOTICE}` : '';
    if (failure === undefined) {
      this.say(`Moved ${what(moved)} to ${place}.${why}`, false, undo);
    } else if (moved.length === 0 && undos.length === 0) {
      this.say(failure, true);
    } else {
      this.say(`Some items were moved to ${place}, but not all. ${failure}${why}`, true, undo);
    }
  }

  /** Takes back what the card tells of: a move to the Trash, or a rename (its undo is a job, which tells how it went when it ends). */
  async undo() {
    const shown = this.toast;
    const undo = shown?.undo;
    if (shown === undefined || undo === null || undo === undefined) {
      return;
    }

    this.toast = { ...shown, undo: null }; // a second click has nothing to undo

    try {
      if (undo.kind === 'trash') {
        const count = await this.#ipc.invoke('ops_untrash', { token: undo.token });
        this.say(`Restored ${count === 1 ? '1 item' : `${count} items`}.`, false);
      } else if (undo.kind === 'rename') {
        await this.renames.undo(this.#ipc, undo.job);
      } else {
        await this.syncs.undo(this.#ipc, undo.job);
      }
    } catch (error) {
      const failure = IpcFailure.from(error);
      // The backend keeps the record when something is in the way, the restore failed, it was busy or the job still runs: Undo again.
      const again = failure.code === 'invalidInput' || failure.code === 'io' || failure.code === 'busy';

      this.say(failure.message, true, again ? undo : null);
    }

    this.#settle();
  }

  say(text: string, error: boolean, undo: ToastUndo | null = null) {
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
