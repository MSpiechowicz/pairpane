import { describe, expect, it } from 'vitest';

import { isTyping, shortcutFor } from './shortcuts';

const press = (key: string, init: KeyboardEventInit = {}) =>
  shortcutFor(new KeyboardEvent('keydown', { key, ...init }));

describe('shortcutFor', () => {
  it('Ctrl+T, Ctrl+W and Ctrl+D', () => {
    expect(press('t', { ctrlKey: true })).toBe('newTab');
    expect(press('T', { ctrlKey: true })).toBe('newTab'); // caps lock
    expect(press('w', { ctrlKey: true })).toBe('closeTab');
    expect(press('d', { ctrlKey: true })).toBe('hotlist');
  });

  it('Ctrl+Tab and Ctrl+Shift+Tab cycle the tabs; Tab alone is not a shortcut', () => {
    expect(press('Tab', { ctrlKey: true })).toBe('nextTab');
    expect(press('Tab', { ctrlKey: true, shiftKey: true })).toBe('previousTab');
    expect(press('Tab')).toBeUndefined();
    expect(press('Tab', { shiftKey: true })).toBeUndefined();
  });

  it('Alt+Left and Alt+Right walk the history, Alt+F1 and Alt+F2 open the drive menus', () => {
    expect(press('ArrowLeft', { altKey: true })).toBe('back');
    expect(press('ArrowRight', { altKey: true })).toBe('forward');
    expect(press('F1', { altKey: true })).toBe('driveMenuLeft');
    expect(press('F2', { altKey: true })).toBe('driveMenuRight');
  });

  it('plain arrows and letters are the pane`s, not shortcuts', () => {
    expect(press('ArrowLeft')).toBeUndefined();
    expect(press('t')).toBeUndefined();
    expect(press('F1')).toBeUndefined();
  });

  it('other modifiers are not shortcuts: Cmd, Shift, and AltGr (Ctrl+Alt) which types characters', () => {
    expect(press('t', { metaKey: true })).toBeUndefined();
    expect(press('w', { metaKey: true, ctrlKey: true })).toBeUndefined();
    expect(press('t', { ctrlKey: true, shiftKey: true })).toBeUndefined();
    expect(press('ArrowLeft', { altKey: true, shiftKey: true })).toBeUndefined();
    expect(press('t', { ctrlKey: true, altKey: true })).toBeUndefined();
    expect(press('ArrowLeft', { ctrlKey: true, altKey: true })).toBeUndefined();
  });

  it('other Ctrl and Alt keys are left alone', () => {
    expect(press('a', { ctrlKey: true })).toBeUndefined();
    expect(press('ArrowLeft', { ctrlKey: true })).toBeUndefined();
    expect(press('x', { altKey: true })).toBeUndefined();
  });
});

describe('isTyping', () => {
  it('is true inside text fields only', () => {
    const input = document.createElement('input');
    const area = document.createElement('textarea');
    const button = document.createElement('button');

    expect(isTyping(input)).toBe(true);
    expect(isTyping(area)).toBe(true);
    expect(isTyping(button)).toBe(false);
    expect(isTyping(document.body)).toBe(false);
    expect(isTyping(null)).toBe(false);
  });
});
