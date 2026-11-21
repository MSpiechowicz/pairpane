import { tick } from 'svelte';

import type { EditOpened } from '$lib/ipc/bindings/EditOpened';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { focusActiveList } from '$lib/panes/focus';
import { formatSize } from '$lib/panes/format';
import type { Target, Under } from '$lib/viewer/viewer.svelte';

import { EditSession } from './session.svelte';

/** What the user answers a question with. */
export type Answer = 'confirm' | 'alternate' | 'cancel';

/** How a save that something asked for ended: written, not written (refused, or given up), or its question was replaced. */
type Written = 'saved' | 'failed' | 'replaced';

/** What may be done with the buffer: thrown away, kept, or the question about it was replaced by another one. */
type Discard = 'go' | 'stay' | 'replaced';

/** A question the editor asks: up to three buttons (the third is Cancel, and so is Escape). */
export interface Question {
  title: string;
  message?: string;
  confirm: string;
  alternate?: string;
  /** The first button cannot be taken back: the focus starts on Cancel. */
  danger?: boolean;
  answer: (answer: Answer) => void;
}

export interface EditorHooks {
  /** What the active pane's cursor is on. */
  under: () => Under;
  /** Tells the user something in a card. */
  say: (text: string, error: boolean) => void;
  /** Shows a file in the viewer (when it is too large to edit). */
  view: (target: Target, notice: string) => void;
}

/**
 * F4: the file under the cursor goes to the external editor when one is set up, else into the built-in one, an
 * overlay of the whole window. Holds the one open {@link EditSession} and asks before anything throws its changes away.
 */
export class Editor {
  session = $state.raw<EditSession | undefined>();
  question = $state.raw<Question | undefined>();

  readonly #ipc: Transport;
  readonly #hooks: EditorHooks;
  #opening = false;
  /** The save that is under way (with whatever it asks), so that a second request joins it instead of being lost. */
  #saving: Promise<Written> | undefined;
  /** Ends the question on the screen as replaced (nobody is left waiting for it). */
  #replace: (() => void) | undefined;

  constructor(ipc: Transport, hooks: EditorHooks) {
    this.#ipc = ipc;
    this.#hooks = hooks;
  }

  /** The overlay has the keyboard: the shortcuts and Tab are its own then. */
  get modal(): boolean {
    return this.session !== undefined;
  }

  get dirty(): boolean {
    return this.session?.dirty === true;
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
    if (this.#opening || this.session !== undefined) {
      return;
    }

    this.#opening = true;

    try {
      const { path, name } = target;
      if (!builtin && (await this.#ipc.invoke('edit_external', { path, name })) === 'launched') {
        return;
      }

      const session = new EditSession(this.#ipc, path, name, (dirty) => this.#publish(dirty));
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
      this.#hooks.view(target, `${target.name} is too large to edit (${formatSize(opened.size)}); it is shown here instead.`);
    } else {
      this.#hooks.say(`${target.name} is not a text file; F3 views it.`, true);
    }
  }

  /** The backend is told whether a window close would lose changes, so that it asks before it closes. */
  #publish(dirty: boolean) {
    this.#ipc.invoke('editor_set_dirty', { dirty }).catch(() => {});
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
    return (await this.#start()) === 'saved';
  }

  #start(): Promise<Written> {
    const session = this.session;
    if (session === undefined) {
      return Promise.resolve('failed');
    }

    if (this.#saving === undefined) {
      const saving = this.#write(session).finally(() => {
        if (this.#saving === saving) {
          this.#saving = undefined;
        }
      });
      this.#saving = saving;
    }

    return this.#saving;
  }

  async #write(session: EditSession): Promise<Written> {
    let result = await session.save(false);
    if (result === 'failed' && session.failure === undefined) {
      // A refusal that came before anything was sent still has to be seen (a save on the way out never ends silently).
      session.failure = `${session.name} was not saved.`;
    }

    if (result === 'changed') {
      return this.#overChanged(session);
    }

    return result === 'saved' ? 'saved' : 'failed';
  }

  /** Asks what to do with a file that changed (or went) since it was read. */
  async #overChanged(session: EditSession): Promise<Written> {
    const gone = session.gone;
    const answer = await this.#ask({
      title: gone ? 'The file was removed' : 'The file changed on disk',
      message: gone
        ? `${session.name} was removed by something else since it was opened.`
        : `${session.name} was changed by something else since it was opened.`,
      confirm: gone ? 'Save anyway' : 'Overwrite',
      alternate: gone ? undefined : 'Reload',
      danger: !gone,
    });

    if (answer === 'replaced') {
      return 'replaced';
    }

    if (answer === 'confirm') {
      return (await session.save(true)) === 'saved' ? 'saved' : 'failed';
    }

    if (answer === 'alternate') {
      await this.reload();
    }

    return 'failed';
  }

  /** A question that is replaced by another is resolved as `replaced`, not as cancelled: its caller must not act. */
  #ask(question: Omit<Question, 'answer'>): Promise<Answer | 'replaced'> {
    this.#replace?.();

    return new Promise((resolve) => {
      const done = () => {
        if (this.question === asked) {
          this.question = undefined;
          this.#replace = undefined;
        }
      };
      const asked: Question = {
        ...question,
        answer: (answer) => {
          done();
          resolve(answer);
        },
      };

      this.#replace = () => {
        done();
        resolve('replaced');
      };
      this.question = asked;
    });
  }

  /** Esc and the Close button: closes, after asking when there are changes. */
  async requestClose() {
    if (this.session !== undefined && (await this.#discard()) === 'go') {
      this.close();
    }
  }

  /**
   * Resolves to whether whatever is about to throw the buffer away (a restart) may go on: at once when nothing is
   * changed, else when the user chose Save (and it saved) or Don't save. A question replaced by another is not a yes.
   */
  async confirmDiscard(): Promise<boolean> {
    return (await this.#discard()) === 'go';
  }

  /**
   * The question is asked every time and never waits on a save that is under way: "Don't save" always gets the user
   * out, even when a write hangs.
   */
  async #discard(): Promise<Discard> {
    const session = this.session;
    if (session === undefined || !session.dirty) {
      return 'go';
    }

    const answer = await this.#ask({
      title: `Save changes to ${session.name}?`,
      message: 'Your changes will be lost if you do not save them.',
      confirm: 'Save',
      alternate: "Don't save",
    });

    if (answer === 'replaced') {
      return 'replaced';
    }

    if (answer === 'confirm') {
      return this.#saveThenGo(session);
    }

    return answer === 'alternate' ? 'go' : 'stay';
  }

  /**
   * Text typed while the save was under way is not in the file: it is saved once more, and whatever is still
   * not saved after that keeps the editor open. Nothing typed is ever thrown away by a Save.
   */
  async #saveThenGo(session: EditSession): Promise<Discard> {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const written = await this.#start();
      if (written !== 'saved') {
        return written === 'replaced' ? 'replaced' : 'stay';
      }

      if (this.session !== session) {
        return 'stay';
      }

      if (!session.dirty) {
        return 'go';
      }
    }

    return 'stay';
  }

  /** Closes the overlay; the list that had the focus takes it back. */
  close() {
    this.session?.release();
    this.session = undefined;
    // A save that never answered is not waited for by the next file.
    this.#saving = undefined;
    void tick().then(focusActiveList);
  }

  /** The window was asked to close while a buffer has changes: it closes once the user has dealt with them. */
  async onCloseRequested() {
    // The question may take as long as the user wants: the backend is told it is being asked, so it does not give up.
    this.#ipc.invoke('editor_close_ack', undefined).catch(() => {});

    const discard = await this.#discard();
    if (discard === 'replaced') {
      // A later close request has its own question; only that one's answer acts.
      return;
    }

    if (discard === 'go') {
      await this.#ipc.invoke('app_close_confirmed', undefined).catch(() => {});

      return;
    }

    // Kept open: telling the backend again that there are changes ends its pending close, so a later close asks
    // again (an unanswered one is given up after a while and would let the window close without asking).
    if (this.dirty) {
      this.#ipc.invoke('editor_set_dirty', { dirty: true }).catch(() => {});
    }
  }
}
