import { describe, expect, it } from 'vitest';

import { isTyping, reloadsPage, shortcutFor } from './shortcuts';

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

  it('F5 copies, F6 moves, Shift+F6 renames and Ctrl+J toggles the transfers', () => {
    expect(press('F5')).toBe('copy');
    expect(press('F6')).toBe('move');
    expect(press('F6', { shiftKey: true })).toBe('rename');
    expect(press('j', { ctrlKey: true })).toBe('transfers');
    expect(press('J', { ctrlKey: true })).toBe('transfers'); // caps lock
  });

  it('other function keys and modified F5 and F6 are not shortcuts', () => {
    expect(press('F3')).toBeUndefined();
    expect(press('F5', { shiftKey: true })).toBeUndefined();
    expect(press('F5', { ctrlKey: true })).toBeUndefined();
    expect(press('F6', { altKey: true })).toBeUndefined();
    expect(press('F6', { metaKey: true })).toBeUndefined();
    expect(press('j')).toBeUndefined();
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

describe('reloadsPage', () => {
  it('is F5 with or without a modifier, and nothing else', () => {
    const reloads = (key: string, init: KeyboardEventInit = {}) =>
      reloadsPage(new KeyboardEvent('keydown', { key, ...init }));

    expect(reloads('F5')).toBe(true);
    expect(reloads('F5', { ctrlKey: true })).toBe(true);
    expect(reloads('F5', { shiftKey: true })).toBe(true);
    expect(reloads('F6')).toBe(false);
    expect(reloads('r')).toBe(false);
  });
});
