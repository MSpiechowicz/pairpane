import { beforeEach, describe, expect, it, vi } from 'vitest';

import { Theme } from './theme.svelte';

/** A window whose OS preference can be flipped, with real (happy-dom) storage and <html>. */
function fakeWindow(prefersLight: boolean) {
  let listener: ((event: { matches: boolean }) => void) | undefined;
  const query = {
    matches: prefersLight,
    addEventListener: (_type: string, fn: typeof listener) => (listener = fn),
  };
  const win = {
    document,
    localStorage: window.localStorage,
    matchMedia: vi.fn(() => query),
  } as unknown as Window;

  return { win, osChanges: (light: boolean) => listener?.({ matches: light }) };
}

describe('Theme', () => {
  beforeEach(() => {
    window.localStorage.clear();
    delete document.documentElement.dataset.theme;
  });

  it('follows the OS by default and when the OS changes', () => {
    const { win, osChanges } = fakeWindow(true);
    const theme = new Theme(win);

    expect(theme.mode).toBe('system');
    expect(document.documentElement.dataset.theme).toBe('light');

    osChanges(false);
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('cycles System → Light → Dark → System and remembers the choice', () => {
    const { win } = fakeWindow(false);
    const theme = new Theme(win);

    theme.cycle();
    expect(theme.mode).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');

    theme.cycle();
    expect(theme.mode).toBe('dark');
    expect(new Theme(win).mode).toBe('dark');

    theme.cycle();
    expect(theme.mode).toBe('system');
  });

  it('a fixed choice ignores OS changes', () => {
    const { win, osChanges } = fakeWindow(false);
    new Theme(win).set('dark');

    osChanges(true);
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('falls back to System for an unknown saved value and when storage throws', () => {
    window.localStorage.setItem('pairpane.theme', 'neon');
    expect(new Theme(fakeWindow(false).win).mode).toBe('system');

    const { win } = fakeWindow(false);
    Object.defineProperty(win, 'localStorage', {
      get() {
        throw new Error('blocked');
      },
    });
    const theme = new Theme(win);
    expect(theme.mode).toBe('system');
    expect(() => theme.set('light')).not.toThrow();
    expect(document.documentElement.dataset.theme).toBe('light');
  });
});
