import { describe, expect, it } from 'vitest';

import { Keymap } from './keymap.svelte';
import { chordOf, refusalOf } from './shortcuts';

const press = (key: string, init: KeyboardEventInit = {}) => new KeyboardEvent('keydown', { key, ...init });

describe('Keymap', () => {
  it('starts with the default keys', () => {
    const keymap = new Keymap();

    expect(keymap.shortcutFor(press('F5'))).toBe('copy');
    expect(keymap.keysOf('palette')).toBe('Ctrl+K');
    expect(keymap.overrides).toEqual([]);
  });

  it('gives an action a new key in place of its own, and saves it', () => {
    const keymap = new Keymap();

    expect(keymap.set('copy', { key: 'y', ctrl: true })).toEqual({ from: undefined });

    expect(keymap.shortcutFor(press('y', { ctrlKey: true }))).toBe('copy');
    expect(keymap.shortcutFor(press('F5'))).toBeUndefined();
    expect(keymap.isCustom('copy')).toBe(true);
    expect(keymap.overrides).toEqual([{ action: 'copy', keys: [{ key: 'y', ctrl: true, alt: false, shift: false }] }]);
  });

  it('takes a key from the action that had it, and says which; a reset gives the defaults back but not the key taken', () => {
    const keymap = new Keymap();

    expect(keymap.set('copy', { key: 'F6' })).toEqual({ from: 'move' });
    expect(keymap.shortcutFor(press('F6'))).toBe('copy');
    expect(keymap.keysOf('move')).toBeUndefined();

    keymap.reset('move');
    expect(keymap.shortcutFor(press('F6'))).toBe('copy');

    keymap.resetAll();
    expect(keymap.shortcutFor(press('F6'))).toBe('move');
    expect(keymap.shortcutFor(press('F5'))).toBe('copy');
  });

  it('refuses a key that would type or move the cursor, and Escape', () => {
    const keymap = new Keymap();

    expect(keymap.set('copy', { key: 'c' })).toMatch(/Ctrl or Alt/);
    expect(keymap.set('copy', { key: 'Escape', ctrl: true })).toMatch(/Escape/);
    expect(keymap.shortcutFor(press('F5'))).toBe('copy');
  });

  it('brings back what was saved, leaving out unknown actions and keys an action cannot have', () => {
    const keymap = new Keymap();

    keymap.restore([
      { action: 'nothing', keys: [{ key: 'x', ctrl: true, alt: false, shift: false }] },
      { action: 'trash', keys: [] },
      { action: 'copy', keys: [{ key: 'a', ctrl: false, alt: false, shift: false }] },
    ]);

    expect(keymap.shortcutFor(press('F8'))).toBeUndefined();
    expect(keymap.keysOf('copy')).toBeUndefined();
    expect(keymap.overrides.map(({ action }) => action)).toEqual(['trash', 'copy']);
  });
});

describe('chordOf and refusalOf', () => {
  it('reads the key and its modifiers, a letter in lower case; a modifier alone, Cmd and AltGr are none', () => {
    expect(chordOf(press('Y', { ctrlKey: true, shiftKey: true }))).toEqual({ key: 'y', ctrl: true, alt: undefined, shift: true });
    expect(chordOf(press('Control', { ctrlKey: true }))).toBeUndefined();
    expect(chordOf(press('k', { metaKey: true }))).toBeUndefined();
    expect(chordOf(press('q', { ctrlKey: true, altKey: true }))).toBeUndefined();
  });

  it('takes a function key or Delete alone, anything with Ctrl or Alt', () => {
    expect(refusalOf({ key: 'F12' })).toBeUndefined();
    expect(refusalOf({ key: 'Delete', shift: true })).toBeUndefined();
    expect(refusalOf({ key: 'x', alt: true })).toBeUndefined();
    expect(refusalOf({ key: 'Enter' })).toBeDefined();
    expect(refusalOf({ key: 'F13' })).toBeDefined();
  });
});
