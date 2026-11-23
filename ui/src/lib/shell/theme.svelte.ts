import type { ThemeChoice } from '$lib/ipc/bindings/ThemeChoice';

import { type Preferences, preferences } from './preferences.svelte';

/** What the user picked; `system` follows the OS. */
export type ThemeMode = ThemeChoice;

const MODES: readonly ThemeMode[] = ['system', 'light', 'dark'];
const STORAGE_KEY = 'pairpane.theme';
const LIGHT_QUERY = '(prefers-color-scheme: light)';

/** Storage can be unavailable or throw (private mode, blocked site data): the theme then just isn't remembered. */
function load(storage: Storage | undefined): ThemeMode {
  try {
    const saved = storage?.getItem(STORAGE_KEY);
    return MODES.find((mode) => mode === saved) ?? 'system';
  } catch {
    return 'system';
  }
}

/**
 * Light or dark, by the user's choice or, by default, the OS. Applies itself as `data-theme` on <html>; the
 * colors live in lib/design/tokens.css.
 *
 * The choice is a preference, kept with the session. The page cannot wait for the session, so the last choice is also
 * kept in the page's own storage: it is what shows before the session is restored (no flash of the other theme), and
 * the restored preference then replaces it.
 */
export class Theme {
  #prefersLight = $state(false);

  readonly #preferences: Preferences;
  readonly #root: HTMLElement;
  readonly #storage: Storage | undefined;

  constructor(win: Window = window, prefs: Preferences = preferences) {
    this.#preferences = prefs;
    this.#root = win.document.documentElement;
    this.#storage = (() => {
      try {
        return win.localStorage;
      } catch {
        return undefined;
      }
    })();
    prefs.set({ theme: load(this.#storage) });

    const query = win.matchMedia(LIGHT_QUERY);
    this.#prefersLight = query.matches;
    query.addEventListener('change', (event) => {
      this.#prefersLight = event.matches;
      this.#apply();
    });

    this.#apply();
  }

  /** What the user picked. */
  get mode(): ThemeMode {
    return this.#preferences.value.theme;
  }

  /** The theme in effect. */
  get resolved(): 'light' | 'dark' {
    if (this.mode === 'system') {
      return this.#prefersLight ? 'light' : 'dark';
    }

    return this.mode;
  }

  /** System → Light → Dark → System. */
  cycle(): void {
    this.set(MODES[(MODES.indexOf(this.mode) + 1) % MODES.length]);
  }

  set(mode: ThemeMode): void {
    this.#preferences.set({ theme: mode });
    this.#remember(mode);
    this.#apply();
  }

  /**
   * Keeps what is shown in step with the preference when something else changes it (the session restoring it, the
   * settings). Call while a component is being set up: it starts an effect.
   */
  watch(): void {
    $effect(() => {
      this.#remember(this.mode);
      this.#apply();
    });
  }

  #remember(mode: ThemeMode): void {
    try {
      this.#storage?.setItem(STORAGE_KEY, mode);
    } catch {
      // Not remembered; it still applies for this session.
    }
  }

  #apply(): void {
    this.#root.dataset.theme = this.resolved;
  }
}
