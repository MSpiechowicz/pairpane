/** What the user picked; `system` follows the OS. */
export type ThemeMode = 'system' | 'light' | 'dark';

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
 */
export class Theme {
  mode = $state<ThemeMode>('system');
  #prefersLight = $state(false);

  readonly #root: HTMLElement;
  readonly #storage: Storage | undefined;

  constructor(win: Window = window) {
    this.#root = win.document.documentElement;
    this.#storage = (() => {
      try {
        return win.localStorage;
      } catch {
        return undefined;
      }
    })();
    this.mode = load(this.#storage);

    const query = win.matchMedia(LIGHT_QUERY);
    this.#prefersLight = query.matches;
    query.addEventListener('change', (event) => {
      this.#prefersLight = event.matches;
      this.#apply();
    });

    this.#apply();
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
    this.mode = mode;
    try {
      this.#storage?.setItem(STORAGE_KEY, mode);
    } catch {
      // Not remembered; it still applies for this session.
    }
    this.#apply();
  }

  #apply(): void {
    this.#root.dataset.theme = this.resolved;
  }
}
