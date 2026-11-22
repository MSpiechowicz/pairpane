import type { CommandArgs, CommandName, CommandResult } from '$lib/ipc/commands';
import { IpcFailure, type Transport } from '$lib/ipc/transport';
import { archiveName } from '$lib/panes/archive';

/** What the commands that read inside an archive ask for; the others never need its password. */
const GUARDED: ReadonlySet<string> = new Set<CommandName>(['archive_open', 'ops_extract', 'archive_view_open']);

/** The password the dialog asks for: whose it is, why it is asked, and why the last one was not taken. */
export interface Prompt {
  /** The archive file. */
  archive: string;
  /** The file name, for the title. */
  name: string;
  /** `needed` for the first ask, `wrong` when the password that was given did not open the archive. */
  reason: 'needed' | 'wrong';
  /** Something other than a wrong password went wrong when it was checked. */
  failure: string | undefined;
  busy: boolean;
}

/** An ask that waits for the dialog: every action that needs the archive joins it and goes on with its answer. */
interface Ask {
  prompt: Prompt;
  /** Called with `true` when the backend has the password, `false` when the user gave up. */
  settle: (taken: boolean) => void;
  answered: Promise<boolean>;
}

/**
 * Asks for the password of an archive when a command that reads it says it needs one, and goes on with that
 * command. The password is typed into the dialog, sent to the backend (which keeps it for the run) and is not
 * kept here: nothing but the question is.
 */
export class Passwords {
  /** The question on the screen: one archive at a time, the others wait. */
  prompt = $state.raw<Prompt | undefined>();

  readonly #ipc: Transport;
  readonly #asks: Ask[] = [];

  constructor(ipc: Transport) {
    this.#ipc = ipc;
  }

  /** A dialog has the keyboard. */
  get modal(): boolean {
    return this.prompt !== undefined;
  }

  /**
   * `ipc` with the guarded commands retried after the user gave the password: a command that fails because the
   * archive needs a password (or the one given does not open it) waits for the dialog and runs again. Giving up
   * is the command's own failure.
   */
  wrap(ipc: Transport): Transport {
    return {
      // The other commands are the transport's own, as they are.
      invoke: (name, args) => (GUARDED.has(name) ? this.#retrying(ipc, name, args) : ipc.invoke(name, args)),
    };
  }

  async #retrying<K extends CommandName>(ipc: Transport, name: K, args: CommandArgs<K>): Promise<CommandResult<K>> {
    const { archive } = args as unknown as { archive: string };

    for (;;) {
      try {
        return await ipc.invoke(name, args);
      } catch (error) {
        const failure = IpcFailure.from(error);
        if (failure.code !== 'passwordRequired' || !(await this.#ask(archive, failure.message))) {
          throw failure;
        }
      }
    }
  }

  /** Waits for the password of `archive`; a question already on the screen for it is joined. */
  #ask(archive: string, message: string): Promise<boolean> {
    const open = this.#asks.find((ask) => ask.prompt.archive === archive);
    if (open !== undefined) {
      return open.answered;
    }

    let settle: (taken: boolean) => void = () => {};
    const answered = new Promise<boolean>((resolve) => (settle = resolve));
    const reason = /wrong/i.test(message) ? 'wrong' : 'needed';
    this.#asks.push({ prompt: { archive, name: archiveName(archive), reason, failure: undefined, busy: false }, settle, answered });
    this.#show();

    return answered;
  }

  #show() {
    this.prompt = this.#asks[0]?.prompt;
  }

  /** Sends what was typed; the archive opens (and the commands go on) when the backend takes it, else it is asked again. */
  async submit(password: string) {
    const ask = this.#asks[0];
    if (ask === undefined || ask.prompt.busy) {
      return;
    }

    this.#update(ask, { busy: true, failure: undefined });
    try {
      const taken = await this.#ipc.invoke('archive_password', { archive: ask.prompt.archive, password });
      if (taken) {
        this.#finish(ask, true);
      } else {
        this.#update(ask, { busy: false, reason: 'wrong' });
      }
    } catch (error) {
      this.#update(ask, { busy: false, failure: IpcFailure.from(error).message });
    }
  }

  /** The user gave up: the commands that waited fail as they would have. */
  cancel() {
    const ask = this.#asks[0];
    if (ask !== undefined && !ask.prompt.busy) {
      this.#finish(ask, false);
    }
  }

  #update(ask: Ask, change: Partial<Prompt>) {
    ask.prompt = { ...ask.prompt, ...change };
    if (this.#asks[0] === ask) {
      this.#show();
    }
  }

  #finish(ask: Ask, taken: boolean) {
    this.#asks.splice(this.#asks.indexOf(ask), 1);
    ask.settle(taken);
    this.#show();
  }
}
