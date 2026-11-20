/** The key under which the version that last played the splash is remembered. */
export const STORAGE_KEY = 'pairpane.splash';

const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)';

/** The browser's storage; it can be unavailable (blocked site data), in which case nothing is remembered. */
export function browserStorage(win: Window = window): Storage | undefined {
  try {
    return win.localStorage;
  } catch {
    return undefined;
  }
}

/** Whether the user asked the system to cut down on motion. */
export function prefersReducedMotion(win: Window = window): boolean {
  try {
    return win.matchMedia(REDUCED_MOTION_QUERY).matches;
  } catch {
    return false;
  }
}

/**
 * The splash plays on the first launch and after an update (the stored version is missing or differs), never
 * under reduced motion. Storage that throws counts as "not seen yet": the splash plays rather than being lost.
 */
export function shouldPlay(storage: Storage | undefined, version: string, reducedMotion: boolean): boolean {
  if (reducedMotion) {
    return false;
  }

  try {
    return storage?.getItem(STORAGE_KEY) !== version;
  } catch {
    return true;
  }
}

/** Remembers that this version played the splash. Storage that throws just means it is not remembered. */
export function markPlayed(storage: Storage | undefined, version: string): void {
  try {
    storage?.setItem(STORAGE_KEY, version);
  } catch {
    // Not remembered; the splash plays again on the next start.
  }
}
