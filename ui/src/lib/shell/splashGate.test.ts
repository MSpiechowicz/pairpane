import { describe, expect, it } from 'vitest';

import { browserStorage, markPlayed, shouldPlay, STORAGE_KEY } from './splashGate';

/** A storage whose every access throws, like site data that is blocked. */
const brokenStorage = {
  getItem: () => {
    throw new Error('blocked');
  },
  setItem: () => {
    throw new Error('blocked');
  },
} as unknown as Storage;

describe('shouldPlay', () => {
  it('plays on the first launch, when nothing is stored', () => {
    expect(shouldPlay(window.localStorage, '0.5.1', false)).toBe(true);
  });

  it('does not play again for a version that already played', () => {
    window.localStorage.setItem(STORAGE_KEY, '0.5.1');

    expect(shouldPlay(window.localStorage, '0.5.1', false)).toBe(false);
  });

  it('plays again after an update', () => {
    window.localStorage.setItem(STORAGE_KEY, '0.5.1');

    expect(shouldPlay(window.localStorage, '0.6.0', false)).toBe(true);
  });

  it('does not play under reduced motion, not even on the first launch', () => {
    window.localStorage.clear();

    expect(shouldPlay(window.localStorage, '0.5.1', true)).toBe(false);
  });

  it('plays when storage throws or is missing, instead of being lost', () => {
    expect(shouldPlay(brokenStorage, '0.5.1', false)).toBe(true);
    expect(shouldPlay(undefined, '0.5.1', false)).toBe(true);
  });
});

describe('markPlayed', () => {
  it('remembers the version, so the next start of it does not play', () => {
    window.localStorage.clear();

    markPlayed(window.localStorage, '0.5.1');

    expect(shouldPlay(window.localStorage, '0.5.1', false)).toBe(false);
  });

  it('survives storage that throws or is missing', () => {
    expect(() => markPlayed(brokenStorage, '0.5.1')).not.toThrow();
    expect(() => markPlayed(undefined, '0.5.1')).not.toThrow();
  });
});

describe('browserStorage', () => {
  it('is undefined when the window refuses access to its storage', () => {
    const win = {
      get localStorage(): Storage {
        throw new Error('blocked');
      },
    } as unknown as Window;

    expect(browserStorage(win)).toBeUndefined();
    expect(browserStorage()).toBe(window.localStorage);
  });
});
