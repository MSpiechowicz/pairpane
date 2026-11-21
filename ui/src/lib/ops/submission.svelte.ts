import { IpcFailure } from '$lib/ipc/transport';

/**
 * What a dialog shows while it asks the backend for something: that it is busy (the buttons are off and Escape
 * waits) and why the last try was refused (the dialog stays open for another).
 */
export class Submission {
  busy = $state(false);
  failure = $state<string | undefined>();

  readonly #leave: () => void;

  /** `leave` closes the dialog. */
  constructor(leave: () => void) {
    this.#leave = leave;
  }

  /** Escape and Cancel; a job that is being started cannot be called off from here (its owner needs the answer). */
  readonly close = () => {
    if (!this.busy) {
      this.#leave();
    }
  };

  /** Runs `work` unless another is running; a refusal becomes `failure`. */
  async run(work: () => Promise<void>): Promise<void> {
    if (this.busy) {
      return;
    }

    this.busy = true;
    this.failure = undefined;
    try {
      await work();
    } catch (error) {
      this.failure = IpcFailure.from(error).message;
    } finally {
      this.busy = false;
    }
  }

  /** Shows a refusal the page found itself, before anything was asked. */
  refuse(message: string) {
    this.failure = message;
  }
}

/** Enter in a text field or a checkbox of the node does `submit`, as the dialog's primary button would. */
export function enterSubmits(submit: () => void) {
  return (node: HTMLElement) => {
    const onkeydown = (event: KeyboardEvent) => {
      if (event.key === 'Enter' && !event.isComposing && event.target instanceof HTMLInputElement) {
        event.preventDefault();
        submit();
      }
    };

    node.addEventListener('keydown', onkeydown);
    return () => node.removeEventListener('keydown', onkeydown);
  };
}
