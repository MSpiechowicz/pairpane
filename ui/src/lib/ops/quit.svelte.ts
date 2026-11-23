import type { Discard } from '$lib/editor/saveFlow.svelte';
import type { Unsaved } from '$lib/editor/unsaved';
import { type Preferences, preferences } from '$lib/shell/preferences.svelte';

/**
 * Asking before the app ends while copies or moves are still running (a setting, on by default). The backend asks the
 * page about a close only while the page says it holds something to lose, so while transfers run (and the question is
 * wanted) this says so, as the editor and the diff do for unsaved text; the question itself comes after theirs.
 */
export class QuitGuard {
  /** The question is on the screen. */
  asking = $state(false);

  readonly #unsaved: Unsaved;
  readonly #transfers: { readonly running: boolean };
  readonly #preferences: Preferences;
  /** Ends the question on the screen with an answer. */
  #answer: ((answer: Discard) => void) | undefined;

  constructor(unsaved: Unsaved, transfers: { readonly running: boolean }, prefs: Preferences = preferences) {
    this.#unsaved = unsaved;
    this.#transfers = transfers;
    this.#preferences = prefs;
  }

  /** Whether the end of the app would stop transfers the user wants to be asked about. */
  get #wanted(): boolean {
    return this.#preferences.value.confirmQuitWithTransfers && this.#transfers.running;
  }

  /** Keeps the backend told. Call while a component is being set up: it starts an effect. */
  watch() {
    $effect(() => this.#unsaved.report('transfers', this.#wanted));
    // The last transfer ended while the question was on the screen: nothing is left to ask about, so the close goes on.
    $effect(() => {
      if (this.asking && !this.#wanted) {
        this.answer(true);
      }
    });
  }

  /**
   * Whether the app may end: at once when no transfer runs (or the question is not wanted), else when the user answers
   * Quit. A question replaced by another resolves as `replaced`, so that only the latest one's answer acts.
   */
  async discard(): Promise<Discard> {
    this.#answer?.('replaced');
    this.#answer = undefined;
    this.asking = false;
    if (!this.#wanted) {
      return 'go';
    }

    return new Promise((resolve) => {
      this.#answer = resolve;
      this.asking = true;
    });
  }

  /** What follows an earlier question of a close: asked only when that one let the close go on. */
  async after(earlier: Discard): Promise<Discard> {
    return earlier === 'go' ? this.discard() : earlier;
  }

  /** The user's answer: Quit (`true`) or Cancel. */
  answer(quit: boolean) {
    const answer = this.#answer;
    this.#answer = undefined;
    this.asking = false;
    answer?.(quit ? 'go' : 'stay');
  }
}
