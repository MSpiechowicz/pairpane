import { tick } from 'svelte';

import { type Discard, type Question, SaveFlow } from '$lib/editor/saveFlow.svelte';
import { ENDING_LABELS, EditSession } from '$lib/editor/session.svelte';
import type { Unsaved } from '$lib/editor/unsaved';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { focusActiveList } from '$lib/panes/focus';
import { displayName } from '$lib/panes/format';
import type { Target } from '$lib/viewer/viewer.svelte';

import { HexDiff } from './hexDiff.svelte';
import type { Picked } from './pick';

export type DiffSide = 'left' | 'right';

export interface DiffHooks {
  /** The two files to compare, or why there are none (the active pane's selection and the panes' cursors). */
  pick: () => Picked | string;
  /** Tells the user something in a card. */
  say: (text: string, error: boolean) => void;
  /** Whether another overlay (the editor) is being opened or is open: only one at a time. */
  blocked?: () => boolean;
}

/** The two files of a text diff, each in an {@link EditSession} of its own. */
export interface TextPair {
  left: EditSession;
  right: EditSession;
}

/**
 * Shift+F2: two files side by side in an overlay of the whole window. Two text files are edited in place with the
 * diff following every keystroke (each side is saved on its own); if either is binary, too large to edit or not
 * text in any encoding, both are shown as bytes, read-only. Asks before anything throws changes away.
 */
export class FileDiff {
  /** The two files of a text diff. */
  pair = $state.raw<TextPair | undefined>();
  /** The two files of a hex diff. */
  hex = $state.raw<HexDiff | undefined>();

  readonly #ipc: Transport;
  readonly #unsaved: Unsaved;
  readonly #hooks: DiffHooks;
  readonly #flow: SaveFlow;
  /** The sessions of the comparison that is being opened or is open: what the dirty mark counts. */
  #sessions: EditSession[] = [];
  #opening = false;

  constructor(ipc: Transport, unsaved: Unsaved, hooks: DiffHooks) {
    this.#ipc = ipc;
    this.#unsaved = unsaved;
    this.#hooks = hooks;
    this.#flow = new SaveFlow(ipc, unsaved, {
      reload: (session) => this.reloadSide(session === this.pair?.left ? 'left' : 'right'),
      isOpen: (session) => this.pair?.left === session || this.pair?.right === session,
    });
  }

  get mode(): 'text' | 'hex' | undefined {
    if (this.pair !== undefined) {
      return 'text';
    }

    return this.hex === undefined ? undefined : 'hex';
  }

  /** The overlay has the keyboard: the shortcuts and Tab are its own then. */
  get modal(): boolean {
    return this.mode !== undefined;
  }

  get dirty(): boolean {
    return this.pair?.left.dirty === true || this.pair?.right.dirty === true;
  }

  /** A comparison is being opened or is open (what keeps the editor from opening beside it). */
  get busy(): boolean {
    return this.#opening || this.mode !== undefined;
  }

  /** The question on the screen: about a file that changed on disk, or about changes that would be lost. */
  get question(): Question | undefined {
    return this.#flow.question;
  }

  /** What the user should know about the two files (not about one of them, which its own header says). */
  get notes(): string[] {
    const { pair } = this;
    if (pair === undefined) {
      return [];
    }

    const notes: string[] = [];
    if (pair.left.lineEnding !== pair.right.lineEnding) {
      notes.push(
        `Line endings differ (${ENDING_LABELS[pair.left.lineEnding]} / ${ENDING_LABELS[pair.right.lineEnding]}); they are kept when saving.`,
      );
    }

    if (pair.left.encoding !== pair.right.encoding) {
      notes.push(`Encodings differ (${pair.left.encoding} / ${pair.right.encoding}); they are kept when saving.`);
    }

    return notes;
  }

  /** Shift+F2: compares what the panes pick, or says why it cannot. */
  open() {
    const picked = this.#hooks.pick();
    if (typeof picked === 'string') {
      this.#hooks.say(picked, true);
    } else {
      void this.openPicked(picked);
    }
  }

  /** Opens `picked`: as text when both files read as text, else as bytes. */
  async openPicked(picked: Picked) {
    if (this.busy || this.#hooks.blocked?.() === true) {
      return;
    }

    this.#opening = true;

    try {
      const pair = { left: this.#session(picked.left), right: this.#session(picked.right) };
      this.#sessions = [pair.left, pair.right];
      const opened = await Promise.all([pair.left.open(null), pair.right.open(null)]);

      if (opened.every((answer) => answer.kind === 'text')) {
        this.pair = pair;
      } else {
        this.hex = new HexDiff(this.#ipc, picked.left, picked.right);
      }
    } catch (error) {
      this.#sessions = [];
      this.#hooks.say(IpcFailure.from(error).message, true);
    } finally {
      this.#opening = false;
    }
  }

  #session(target: Target): EditSession {
    return new EditSession(this.#ipc, target.path, target.name, () => this.#unsaved.report('diff', this.#sessions.some((session) => session.dirty)));
  }

  /** Writes one side (Ctrl+S, or its Save button); resolves to whether the file was written. A file that changed on disk is asked about first. */
  async save(side: DiffSide): Promise<boolean> {
    const session = this.pair?.[side];

    return session !== undefined && (await this.#flow.start(session)) === 'saved';
  }

  /** Reads a side's file again (what was changed in it is given up); a file that is no longer text ends the comparison. */
  async reloadSide(side: DiffSide) {
    const session = this.pair?.[side];
    if (session === undefined) {
      return;
    }

    try {
      const opened = await session.open(session.encoding);
      if (opened.kind === 'tooLarge' || opened.kind === 'binary') {
        await this.#leave(session, side === 'left' ? 'right' : 'left');
      }
    } catch (error) {
      session.failure = IpcFailure.from(error).message;
    }
  }

  /**
   * `session` can no longer be compared as text, which ends the comparison. The other side's changes are asked
   * about first; on Cancel the comparison stays, with `session` shown as it was but locked.
   */
  async #leave(session: EditSession, other: DiffSide) {
    session.retire('This file can no longer be compared as text.');

    const kept = this.pair?.[other];
    if (kept === undefined || (await this.#flow.discard([kept])) === 'go') {
      this.close();
      this.#hooks.say(`${displayName(session.name)} is no longer a text file.`, true);
    }
  }

  /** Esc and the Close button: closes, after asking when either side has changes. */
  async requestClose() {
    if ((await this.#flow.discard(this.#sessions)) === 'go') {
      this.close();
    }
  }

  /** Whether whatever is about to throw the changes away (a restart) may go on; see {@link SaveFlow.discard}. */
  async confirmDiscard(): Promise<boolean> {
    return (await this.discard()) === 'go';
  }

  /** The question about the changes of both sides (none to ask about is a `go`); see {@link SaveFlow.discard}. */
  discard(): Promise<Discard> {
    return this.#flow.discard(this.#sessions);
  }

  /**
   * The window was asked to close while a side has changes: it closes once the user has dealt with them (and with
   * whatever `next` asks about after them).
   */
  async onCloseRequested(next?: () => Promise<Discard>) {
    await this.#flow.closeRequested(this.#sessions, next);
  }

  /** Closes the overlay; the list that had the focus takes it back. */
  close() {
    for (const session of this.#sessions) {
      session.release();
      this.#flow.forget(session);
    }

    this.hex?.close();
    this.pair = undefined;
    this.hex = undefined;
    this.#sessions = [];
    void tick().then(focusActiveList);
  }
}
