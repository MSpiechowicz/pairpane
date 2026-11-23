import type { ShellEvent } from '$lib/ipc/bindings/ShellEvent';
import type { ShellId } from '$lib/ipc/bindings/ShellId';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

/** The commands kept for the arrow keys. */
export const MOST_REMEMBERED = 50;

/** The most output kept on the page, in characters (the backend sends at most about as much). */
const MOST_KEPT = 1024 * 1024;

/** Where a command would run: the active pane's folder, or why there is none. */
export type Place = { folder: string } | { refusal: string };

/** Where a command runs for a pane's listing: a folder on disk, not an archive or search results. */
export function placeOf(listing: { folder: string | undefined; readonly: boolean; results: unknown }): Place {
  if (listing.readonly || listing.results !== undefined) {
    return { refusal: 'Commands run in a folder on disk: open one first.' };
  }

  return listing.folder === undefined ? { refusal: 'Wait until the folder has been listed.' } : { folder: listing.folder };
}

/** What the output panel shows of the last command. */
export interface Shown {
  line: string;
  output: string;
  /** `running` until the command ends; then how it ended. */
  status: 'starting' | 'running' | 'done' | 'stopped' | 'failed';
  /** The exit code, when it ended with one. */
  code?: number | null;
  /** Why it failed, or that the output was cut. */
  note?: string;
}

/**
 * The dock's command line: a line runs in the user's shell in the active pane's folder (`shell_run`), its output shown
 * in a panel until it is closed; it can be stopped. The lines run are remembered (in this run only) for the arrow keys.
 * One command shows at a time: another may start only once the shown one has ended. It also opens the terminal there.
 */
export class CommandLine {
  readonly #ipc: Transport;
  readonly #place: () => Place;
  readonly #say: (text: string, error: boolean) => void;

  shown = $state<Shown | undefined>();
  #id: ShellId | undefined;
  /** Bumped by each run, so events of an earlier one are left out. */
  #run = 0;

  #history: string[] = [];
  /** Where the arrow keys are in the history: its length when at the line being typed. */
  #cursor = 0;

  constructor(ipc: Transport, place: () => Place, say: (text: string, error: boolean) => void) {
    this.#ipc = ipc;
    this.#place = place;
    this.#say = say;
  }

  get running(): boolean {
    return this.shown?.status === 'starting' || this.shown?.status === 'running';
  }

  /** Runs `line` in the active folder; `false` when it was not started (the notice says why), and the line is kept. */
  async run(line: string): Promise<boolean> {
    const text = line.trim();
    if (text === '') {
      return false;
    }
    if (this.running) {
      this.#say('A command is running: stop it first.', true);
      return false;
    }

    const place = this.#place();
    if ('refusal' in place) {
      this.#say(place.refusal, true);
      return false;
    }

    this.#remember(text);
    const run = ++this.#run;
    this.#id = undefined;
    this.shown = { line: text, output: '', status: 'starting' };

    try {
      const id = await this.#ipc.invoke('shell_run', {
        path: place.folder,
        command: text,
        onEvent: (event: ShellEvent) => this.#take(run, event),
      });
      if (run !== this.#run) {
        // Closed while it was starting.
        this.#ipc.invoke('shell_stop', { id }).catch(() => {});
      } else if (this.shown !== undefined && this.running) {
        this.#id = id;
        this.shown = { ...this.shown, status: 'running' };
      }
    } catch (error) {
      if (run === this.#run) {
        this.shown = { ...this.shown, status: 'failed', note: messageOf(error) };
      }
    }

    return true;
  }

  /** Stops the command shown (its end follows as an event). */
  stop() {
    if (this.#id !== undefined && this.running) {
      this.#ipc.invoke('shell_stop', { id: this.#id }).catch(() => {});
    }
  }

  /** Hides the output, stopping the command when it still runs. */
  close() {
    this.stop();
    this.#run++;
    this.#id = undefined;
    this.shown = undefined;
  }

  /** The line before (`-1`) or after (`1`) the one the arrow keys are at; `undefined` past either end. */
  recall(step: -1 | 1): string | undefined {
    const next = this.#cursor + step;
    if (next < 0 || next > this.#history.length) {
      return undefined;
    }

    this.#cursor = next;
    return this.#history[next] ?? '';
  }

  /** Opens the user's terminal in the active folder. */
  async terminal() {
    const place = this.#place();
    if ('refusal' in place) {
      this.#say(place.refusal, true);
      return;
    }

    try {
      await this.#ipc.invoke('terminal_open', { path: place.folder });
    } catch (error) {
      this.#say(`Cannot open a terminal: ${messageOf(error)}`, true);
    }
  }

  #remember(line: string) {
    this.#history = [...this.#history.filter((kept) => kept !== line), line].slice(-MOST_REMEMBERED);
    this.#cursor = this.#history.length;
  }

  #take(run: number, event: ShellEvent) {
    const shown = this.shown;
    if (run !== this.#run || shown === undefined) {
      return;
    }

    if (event.kind === 'output') {
      const output = (shown.output + event.text).slice(-MOST_KEPT);
      this.shown = { ...shown, status: shown.status === 'starting' ? 'running' : shown.status, output };
    } else if (event.kind === 'done') {
      this.shown = {
        ...shown,
        status: event.stopped ? 'stopped' : 'done',
        code: event.code,
        note: event.truncated ? 'The output was too long: the rest is left out.' : undefined,
      };
      this.#id = undefined;
    } else {
      this.shown = { ...shown, status: 'failed', note: event.error.message };
      this.#id = undefined;
    }
  }
}

function messageOf(error: unknown): string {
  return error instanceof IpcFailure ? error.message : 'something went wrong';
}
