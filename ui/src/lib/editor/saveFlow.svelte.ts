import type { Transport } from '$lib/ipc/transport';

import type { EditSession } from './session.svelte';
import type { Unsaved } from './unsaved';

/** What the user answers a question with. */
export type Answer = 'confirm' | 'alternate' | 'cancel';

/** How a save that something asked for ended: written, not written (refused, or given up), or its question was replaced. */
export type Written = 'saved' | 'failed' | 'replaced';

/** What may be done with the buffers: thrown away, kept, or the question about them was replaced by another one. */
export type Discard = 'go' | 'stay' | 'replaced';

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

export interface SaveHooks {
  /** Reads `session`'s file again (the answer "Reload" to a file that changed on disk). */
  reload: (session: EditSession) => Promise<void>;
  /** Whether `session` is still the one on screen (a close or a reload in the middle of a save replaces it). */
  isOpen: (session: EditSession) => boolean;
}

/**
 * Saving, and asking before anything throws changes away, for the editor's one session and the diff's two. A save
 * that is under way is joined by a second request; a file that changed on disk is not written over unasked.
 */
export class SaveFlow {
  /** The question on the screen, if any. */
  question = $state.raw<Question | undefined>();

  readonly #ipc: Transport;
  readonly #unsaved: Unsaved;
  readonly #hooks: SaveHooks;
  /** The saves that are under way (with whatever they ask), so that a second request joins one instead of being lost. */
  readonly #saving = new Map<EditSession, Promise<Written>>();
  /** Ends the question on the screen as replaced (nobody is left waiting for it). */
  #replace: (() => void) | undefined;

  constructor(ipc: Transport, unsaved: Unsaved, hooks: SaveHooks) {
    this.#ipc = ipc;
    this.#unsaved = unsaved;
    this.#hooks = hooks;
  }

  /**
   * Writes the file. A file that changed on disk is not written over unasked: the user chooses to overwrite it
   * (or save it again, when it went), to read it again, or to leave it. Asked again while a save is under way, it waits for that one.
   */
  start(session: EditSession): Promise<Written> {
    const joined = this.#saving.get(session);
    if (joined !== undefined) {
      return joined;
    }

    const saving = this.#write(session).finally(() => {
      if (this.#saving.get(session) === saving) {
        this.#saving.delete(session);
      }
    });
    this.#saving.set(session, saving);

    return saving;
  }

  /** A save that never answered is not waited for by what comes next. */
  forget(session: EditSession) {
    this.#saving.delete(session);
  }

  async #write(session: EditSession): Promise<Written> {
    const result = await session.save(false);
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
    const answer = await this.ask({
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
      await this.#hooks.reload(session);
    }

    return 'failed';
  }

  /** A question that is replaced by another is resolved as `replaced`, not as cancelled: its caller must not act. */
  ask(question: Omit<Question, 'answer'>): Promise<Answer | 'replaced'> {
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

  /**
   * The question is asked every time and never waits on a save that is under way: "Don't save" always gets the user
   * out, even when a write hangs. Nothing to ask when no session has changes.
   */
  async discard(sessions: readonly EditSession[]): Promise<Discard> {
    const dirty = sessions.filter((session) => session.dirty);
    if (dirty.length === 0) {
      return 'go';
    }

    const answer = await this.ask({
      title: dirty.length > 1 ? 'Save changes to both files?' : `Save changes to ${dirty[0].name}?`,
      message: 'Your changes will be lost if you do not save them.',
      confirm: 'Save',
      alternate: "Don't save",
    });

    if (answer === 'replaced') {
      return 'replaced';
    }

    if (answer === 'confirm') {
      return this.#saveThenGo(dirty);
    }

    return answer === 'alternate' ? 'go' : 'stay';
  }

  /** Every changed session is written; what is still not saved after that keeps things open. */
  async #saveThenGo(sessions: readonly EditSession[]): Promise<Discard> {
    for (const session of sessions) {
      const kept = await this.#saveOne(session);
      if (kept !== 'go') {
        return kept;
      }
    }

    return 'go';
  }

  /**
   * Text typed while the save was under way is not in the file: it is saved once more, and whatever is still
   * not saved after that keeps the editor open. Nothing typed is ever thrown away by a Save.
   */
  async #saveOne(session: EditSession): Promise<Discard> {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const written = await this.start(session);
      if (written !== 'saved') {
        return written === 'replaced' ? 'replaced' : 'stay';
      }

      if (!this.#hooks.isOpen(session)) {
        return 'stay';
      }

      if (!session.dirty) {
        return 'go';
      }
    }

    return 'stay';
  }

  /**
   * The window was asked to close while something has changes: it closes once the user has dealt with them. The
   * backend is told it is being asked (the question may take as long as the user wants), then the answer is acted on.
   * `next` asks about what another owner of changes holds, once these are dealt with (a Cancel stops there).
   */
  async closeRequested(sessions: readonly EditSession[], next?: () => Promise<Discard>) {
    this.#ipc.invoke('editor_close_ack', undefined).catch(() => {});

    let discard = await this.discard(sessions);
    if (discard === 'go' && next !== undefined) {
      discard = await next();
    }

    if (discard === 'replaced') {
      // A later close request has its own question; only that one's answer acts.
      return;
    }

    if (discard === 'go') {
      await this.#ipc.invoke('app_close_confirmed', undefined).catch(() => {});

      return;
    }

    // Kept open: the backend hears again that there are changes, so a later close asks again.
    this.#unsaved.stillDirty();
  }
}
