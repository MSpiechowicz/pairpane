import { tick } from 'svelte';

import type { EditOpened } from '$lib/ipc/bindings/EditOpened';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { focusActiveList } from '$lib/panes/focus';
import { displayName, formatSize } from '$lib/panes/format';
import type { Target, Under } from '$lib/viewer/viewer.svelte';

import { type Discard, type Question, SaveFlow } from './saveFlow.svelte';
import { EditSession } from './session.svelte';
import { Unsaved } from './unsaved';

export interface EditorHooks {
  /** What the active pane's cursor is on. */
  under: () => Under;
  /** Tells the user something in a card. */
  say: (text: string, error: boolean) => void;
  /** Shows a file in the viewer (when it is too large to edit). */
  view: (target: Target, notice: string) => void;
  /** Whether another overlay (the diff) is being opened or is open: only one at a time. */
  blocked?: () => boolean;
}

/**
 * F4: the file under the cursor goes to the external editor when one is set up, else into the built-in one, an
 * overlay of the whole window. Holds the one open {@link EditSession} and asks before anything throws its changes away.
 */
export class Editor {
  session = $state.raw<EditSession | undefined>();

  readonly #ipc: Transport;
  readonly #hooks: EditorHooks;
  readonly #unsaved: Unsaved;
  readonly #flow: SaveFlow;
  #opening = false;

  constructor(ipc: Transport, hooks: EditorHooks, unsaved: Unsaved = new Unsaved(ipc)) {
    this.#ipc = ipc;
    this.#hooks = hooks;
    this.#unsaved = unsaved;
    this.#flow = new SaveFlow(ipc, unsaved, {
      reload: () => this.reload(),
      isOpen: (session) => this.session === session,
    });
  }

  /** The question on the screen: about a file that changed on disk, or about changes that would be lost. */
  get question(): Question | undefined {
    return this.#flow.question;
  }

  /** The overlay has the keyboard: the shortcuts and Tab are its own then. */
  get modal(): boolean {
    return this.session !== undefined;
  }

  get dirty(): boolean {
    return this.session?.dirty === true;
  }

  /** An editor is being opened or is open (what keeps the diff from opening beside it). */
  get busy(): boolean {
    return this.#opening || this.session !== undefined;
  }

  /** F4 on the active cursor: a folder, `..` and an empty pane have nothing to edit. */
  edit(builtin = false) {
    const target = this.#hooks.under();
    if (typeof target === 'object') {
      void this.open(target, { builtin });
    }
  }

  /** Opens `target`: in the external editor unless `builtin` (or none is set up), else in the overlay. */
  async open(target: Target, { builtin }: { builtin: boolean }) {
    if (this.busy || this.#hooks.blocked?.() === true) {
      return;
    }

    this.#opening = true;

    try {
      const { path, name } = target;
      if (!builtin && (await this.#ipc.invoke('edit_external', { path, name })) === 'launched') {
        return;
      }

      const session = new EditSession(this.#ipc, path, name, (dirty) => this.#unsaved.report('editor', dirty));
      const opened = await session.open(null);
      if (opened.kind === 'text' || opened.kind === 'undecodable') {
        this.session = session;
      } else {
        this.#refuse(target, opened);
      }
    } catch (error) {
      this.#hooks.say(IpcFailure.from(error).message, true);
    } finally {
      this.#opening = false;
    }
  }

  /** What was not opened: a file too large for the editor goes to the viewer, binary content is refused. */
  #refuse(target: Target, opened: EditOpened) {
    if (opened.kind === 'tooLarge') {
      this.#hooks.view(target, `${displayName(target.name)} is too large to edit (${formatSize(opened.size)}); it is shown here instead.`);
    } else {
      this.#hooks.say(`${displayName(target.name)} is not a text file; F3 views it.`, true);
    }
  }

  /** The encoding select: reads the file again in it, or (with changes in the buffer) changes only how it is saved. */
  async changeEncoding(encoding: string) {
    const session = this.session;
    if (session !== undefined) {
      await this.#reread(session, () => session.chooseEncoding(encoding));
    }
  }

  /** Reads the file again, in the encoding it is saved in; what was changed in the buffer is given up. */
  async reload() {
    const session = this.session;
    if (session !== undefined) {
      await this.#reread(session, () => session.open(session.encoding));
    }
  }

  async #reread(session: EditSession, read: () => Promise<EditOpened> | undefined) {
    try {
      const opened = await read();
      if (opened?.kind === 'tooLarge' || opened?.kind === 'binary') {
        this.close();
        this.#refuse({ path: session.path, name: session.name }, opened);
      }
    } catch (error) {
      session.failure = IpcFailure.from(error).message;
    }
  }

  /**
   * Writes the file. A file that changed on disk is not written over unasked: the user chooses to overwrite it
   * (or save it again, when it went), to read it again, or to leave it. Resolves to whether the file was written.
   * Asked again while a save is under way, it waits for that one.
   */
  async save(): Promise<boolean> {
    const session = this.session;

    return session !== undefined && (await this.#flow.start(session)) === 'saved';
  }

  /** Esc and the Close button: closes, after asking when there are changes. */
  async requestClose() {
    const session = this.session;
    if (session !== undefined && (await this.#flow.discard([session])) === 'go') {
      this.close();
    }
  }

  /**
   * Resolves to whether whatever is about to throw the buffer away (a restart) may go on: at once when nothing is
   * changed, else when the user chose Save (and it saved) or Don't save. A question replaced by another is not a yes.
   */
  async confirmDiscard(): Promise<boolean> {
    return (await this.discard()) === 'go';
  }

  /** The question about the buffer's changes (none to ask about is a `go`); see {@link SaveFlow.discard}. */
  async discard(): Promise<Discard> {
    const session = this.session;

    return session === undefined ? 'go' : this.#flow.discard([session]);
  }

  /** Closes the overlay; the list that had the focus takes it back. */
  close() {
    const session = this.session;
    session?.release();
    this.session = undefined;
    if (session !== undefined) {
      // A save that never answered is not waited for by the next file.
      this.#flow.forget(session);
    }
    void tick().then(focusActiveList);
  }

  /**
   * The window was asked to close while a buffer has changes: it closes once the user has dealt with them (and with
   * whatever `next` asks about after them).
   */
  async onCloseRequested(next?: () => Promise<Discard>) {
    await this.#flow.closeRequested(this.session === undefined ? [] : [this.session], next);
  }
}
