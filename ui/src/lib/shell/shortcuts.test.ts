import { describe, expect, it } from 'vitest';

import { isTyping, keysOf, refusalOf, reloadsPage, shortcutFor } from './shortcuts';

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

  it('Alt+Enter shows the properties; Enter alone and the other modifiers do not', () => {
    expect(press('Enter', { altKey: true })).toBe('properties');
    expect(press('Enter')).toBeUndefined();
    expect(press('Enter', { ctrlKey: true })).toBeUndefined();
    expect(press('Enter', { altKey: true, shiftKey: true })).toBeUndefined();
    expect(press('Enter', { altKey: true, ctrlKey: true })).toBeUndefined();
  });

  it('Alt+F9 extracts; F9 alone and the other modifiers do not', () => {
    expect(press('F9', { altKey: true })).toBe('extract');
    expect(press('F9')).toBeUndefined();
    expect(press('F9', { altKey: true, shiftKey: true })).toBeUndefined();
    expect(press('F9', { altKey: true, ctrlKey: true })).toBeUndefined();
  });

  it('Ctrl+M opens the multi-rename; M alone, with Shift or with the other modifiers does not', () => {
    expect(press('m', { ctrlKey: true })).toBe('multiRename');
    expect(press('M', { ctrlKey: true })).toBe('multiRename'); // caps lock
    expect(press('m')).toBeUndefined();
    expect(press('m', { ctrlKey: true, shiftKey: true })).toBeUndefined();
    expect(press('m', { metaKey: true })).toBeUndefined();
    expect(press('m', { ctrlKey: true, altKey: true })).toBeUndefined(); // AltGr types characters
  });

  it('Alt+F5 packs; F5 alone and the other modifiers do not', () => {
    expect(press('F5', { altKey: true })).toBe('pack');
    expect(press('F5')).toBe('copy');
    expect(press('F5', { altKey: true, shiftKey: true })).toBeUndefined();
    expect(press('F5', { altKey: true, ctrlKey: true })).toBeUndefined();
  });

  it('Alt+F7 finds files; F7 alone is a new folder and the other modifiers are not shortcuts', () => {
    expect(press('F7', { altKey: true })).toBe('find');
    expect(press('F7')).toBe('newFolder');
    expect(press('F7', { altKey: true, shiftKey: true })).toBeUndefined();
    expect(press('F7', { altKey: true, ctrlKey: true })).toBeUndefined();
    expect(press('F7', { altKey: true, metaKey: true })).toBeUndefined();
  });

  it('F5 copies, F6 moves, Shift+F6 renames and Ctrl+J toggles the transfers', () => {
    expect(press('F5')).toBe('copy');
    expect(press('F6')).toBe('move');
    expect(press('F6', { shiftKey: true })).toBe('rename');
    expect(press('j', { ctrlKey: true })).toBe('transfers');
    expect(press('J', { ctrlKey: true })).toBe('transfers'); // caps lock
  });

  it('F3 views the file and Ctrl+Q swaps the other pane for a quick view', () => {
    expect(press('F3')).toBe('view');
    expect(press('F3', { shiftKey: true })).toBeUndefined();
    expect(press('F3', { ctrlKey: true })).toBeUndefined();
    expect(press('F3', { altKey: true })).toBeUndefined();
    expect(press('F3', { metaKey: true })).toBeUndefined();
    expect(press('q', { ctrlKey: true })).toBe('quickView');
    expect(press('Q', { ctrlKey: true })).toBe('quickView'); // caps lock
    expect(press('q', { ctrlKey: true, shiftKey: true })).toBeUndefined();
    expect(press('q', { metaKey: true })).toBeUndefined(); // Cmd+Q quits the app on macOS
    expect(press('q')).toBeUndefined();
  });

  it('Ctrl+B lists every file below the folder; B alone and the other modifiers do not', () => {
    expect(press('b', { ctrlKey: true })).toBe('flatView');
    expect(press('B', { ctrlKey: true })).toBe('flatView'); // caps lock
    expect(press('b')).toBeUndefined();
    expect(press('b', { ctrlKey: true, shiftKey: true })).toBeUndefined();
    expect(press('b', { altKey: true })).toBeUndefined();
    expect(press('b', { metaKey: true })).toBeUndefined();
  });

  it('F2 renames in place, F7 makes a folder and Shift+F4 a file', () => {
    expect(press('F2')).toBe('renameInline');
    expect(press('F7')).toBe('newFolder');
    expect(press('F4', { shiftKey: true })).toBe('newFile');
  });

  it('F8 and Delete move to the Trash; with Shift they delete for good', () => {
    expect(press('F8')).toBe('trash');
    expect(press('Delete')).toBe('trash');
    expect(press('F8', { shiftKey: true })).toBe('deletePermanently');
    expect(press('Delete', { shiftKey: true })).toBe('deletePermanently');
  });

  it('plain F4 edits the file', () => {
    expect(press('F4')).toBe('edit');
  });

  it('Shift+F2 compares two files; plain F2 still renames in place', () => {
    expect(press('F2', { shiftKey: true })).toBe('compareFiles');
    expect(press('F2')).toBe('renameInline');
  });

  it('Shift+F7 and the Ctrl, Alt and Cmd variants of the new keys are not shortcuts', () => {
    expect(press('F7', { shiftKey: true })).toBeUndefined();

    for (const name of ['F4', 'F7', 'F8', 'Delete']) {
      for (const modifier of ['ctrlKey', 'metaKey'] as const) {
        expect(press(name, { [modifier]: true })).toBeUndefined();
        expect(press(name, { [modifier]: true, shiftKey: true })).toBeUndefined();
      }
    }

    expect(press('F2', { metaKey: true })).toBeUndefined();
    expect(press('F2', { metaKey: true, shiftKey: true })).toBeUndefined();
    expect(press('F2', { ctrlKey: true, shiftKey: true })).toBeUndefined();

    for (const name of ['F4', 'F8', 'Delete']) {
      expect(press(name, { altKey: true })).toBeUndefined();
      expect(press(name, { altKey: true, shiftKey: true })).toBeUndefined();
    }
  });

  it('Ctrl+F1 shows the pane in brief and Ctrl+F2 in detail; the plain, Shift and Alt keys are the others', () => {
    expect(press('F1', { ctrlKey: true })).toBe('layoutBrief');
    expect(press('F2', { ctrlKey: true })).toBe('layoutDetailed');
    expect(press('F1', { ctrlKey: true, shiftKey: true })).toBeUndefined();
    expect(press('F1', { altKey: true })).toBe('driveMenuLeft');
    expect(press('F2')).toBe('renameInline');
    expect(refusalOf({ key: 'F1', ctrl: true })).toBeUndefined();
    expect(refusalOf({ key: 'F2', ctrl: true })).toBeUndefined();
    expect(keysOf('layoutBrief')).toBe('Ctrl+F1');
    expect(keysOf('layoutDetailed')).toBe('Ctrl+F2');
  });

  it('Alt+F2 is still the right drive menu, not a rename', () => {
    expect(press('F2', { altKey: true })).toBe('driveMenuRight');
  });

  it('other function keys and modified F5 and F6 are not shortcuts', () => {
    expect(press('F9')).toBeUndefined();
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
