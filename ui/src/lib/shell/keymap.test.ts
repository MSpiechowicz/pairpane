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

  it('brings back what was saved, leaving out unknown actions, keys an action cannot have and keys taken already', () => {
    const keymap = new Keymap();
    const chord = (key: string, ctrl = true) => ({ key, ctrl, alt: false, shift: false });

    keymap.restore([
      { action: 'nothing', keys: [chord('x')] },
      { action: 'trash', keys: [] },
      { action: 'copy', keys: [chord('a', false)] },
      { action: 'move', keys: [chord('Y')] },
      { action: 'view', keys: [chord('y'), chord('u')] },
    ]);

    expect(keymap.shortcutFor(press('F8'))).toBeUndefined();
    expect(keymap.keysOf('copy')).toBe('F5');
    expect(keymap.shortcutFor(press('y', { ctrlKey: true }))).toBe('move');
    expect(keymap.allKeysOf('view')).toEqual(['Ctrl+U']);
    expect(keymap.overrides.map(({ action }) => action)).toEqual(['trash', 'move', 'view']);
  });

  it('never takes the only key of the palette, nor keys the panes take first', () => {
    const keymap = new Keymap();

    expect(keymap.set('copy', { key: 'k', ctrl: true })).toMatch(/only key of the command palette/);
    expect(keymap.keysOf('palette')).toBe('Ctrl+K');
    expect(keymap.set('copy', { key: 'a', ctrl: true })).toMatch(/selects every item/);

    keymap.set('palette', { key: 'p', ctrl: true });
    expect(keymap.set('copy', { key: 'p', ctrl: true })).toMatch(/only key of the command palette/);
  });

  it('tells when an action has other keys than its own, also when one was taken from it', () => {
    const keymap = new Keymap();
    expect(keymap.differs('move')).toBe(false);

    keymap.set('copy', { key: 'F6' });

    expect(keymap.differs('move')).toBe(true);
    keymap.reset('copy');
    expect(keymap.differs('copy')).toBe(false);
  });
});

describe('chordOf and refusalOf', () => {
  it('reads the key and its modifiers, a letter in lower case; a modifier alone, Cmd and AltGr are none', () => {
    expect(chordOf(press('Y', { ctrlKey: true, shiftKey: true }))).toEqual({ key: 'y', ctrl: true, alt: undefined, shift: true });
    expect(chordOf(press('Control', { ctrlKey: true }))).toBeUndefined();
    expect(chordOf(press('k', { metaKey: true }))).toBeUndefined();
    expect(chordOf(press('q', { ctrlKey: true, altKey: true }))).toBeUndefined();
  });

  it('refuses Option with a character on macOS, where it types it', () => {
    expect(refusalOf({ key: 'x', alt: true }, true)).toMatch(/macOS/);
    expect(refusalOf({ key: 'F3', alt: true }, true)).toBeUndefined();
  });

  it('takes a function key or Delete alone, anything with Ctrl or Alt', () => {
    expect(refusalOf({ key: 'F12' })).toBeUndefined();
    expect(refusalOf({ key: 'Delete', shift: true })).toBeUndefined();
    expect(refusalOf({ key: 'x', alt: true }, false)).toBeUndefined();
    expect(refusalOf({ key: 'Enter' })).toBeDefined();
    expect(refusalOf({ key: 'F13' })).toBeDefined();
  });
});
