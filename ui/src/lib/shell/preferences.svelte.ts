import type { Preferences as Saved } from '$lib/ipc/bindings/Preferences';
import type { ThemeChoice } from '$lib/ipc/bindings/ThemeChoice';

const THEMES: readonly ThemeChoice[] = ['system', 'light', 'dark'];

/** What a user who never opened the settings has (the same as the backend's defaults). */
export const DEFAULT_PREFERENCES: Readonly<Saved> = {
  theme: 'system',
  showExt: true,
  showSize: true,
  showModified: true,
  showDimensions: false,
  showChecksum: false,
  showGit: false,
  showHidden: false,
  confirmTrash: true,
  confirmTransfer: true,
  confirmQuitWithTransfers: true,
  checkUpdatesAtStart: true,
};

/** The saved preferences with whatever is not usable (a value of another type, an unknown theme) given its default. */
function usable(saved: Partial<Record<keyof Saved, unknown>>): Saved {
  const theme = THEMES.find((choice) => choice === saved.theme) ?? DEFAULT_PREFERENCES.theme;
  const preferences = { ...DEFAULT_PREFERENCES, theme };

  for (const key of Object.keys(DEFAULT_PREFERENCES) as (keyof Saved)[]) {
    const value = saved[key];
    if (key !== 'theme' && typeof value === 'boolean') {
      Object.assign(preferences, { [key]: value });
    }
  }

  return preferences;
}

/**
 * What the user chose in the settings: the theme, the columns the panes show, whether hidden files are listed, which
 * questions are asked and whether the app looks for an update when it starts. A change applies at once to whatever
 * reads {@link value}; the session keeps it with the rest of the state.
 */
export class Preferences {
  #value = $state.raw<Saved>({ ...DEFAULT_PREFERENCES });

  /** The preferences in use. A new object after every change, so a reader (or an effect) sees it. */
  get value(): Readonly<Saved> {
    return this.#value;
  }

  /** Changes what `patch` names. */
  set(patch: Partial<Saved>) {
    this.#value = { ...this.#value, ...patch };
  }

  /**
   * Brings back what was saved. Nothing saved (a first start, a state that could not be read) gives the defaults, but
   * for the theme, which stays what the page already shows (see `Theme`).
   */
  restore(saved?: Partial<Record<keyof Saved, unknown>>) {
    this.#value = saved === undefined ? { ...DEFAULT_PREFERENCES, theme: this.#value.theme } : usable(saved);
  }
}

/** The preferences of the app. */
export const preferences = new Preferences();
