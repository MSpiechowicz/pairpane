import { EditorView } from '@codemirror/view';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createSurface } from './cm';
import type { EditorSurface } from './surface';

const parent = document.body.appendChild(document.createElement('div'));
let surface: EditorSurface | undefined;

function make(text: string, name = 'notes.txt', onSave = () => {}) {
  surface = createSurface({ parent, name, text, readOnly: false, onSave });
  return surface;
}

const view = () => EditorView.findFromDOM(parent)!;

afterEach(() => {
  surface?.destroy();
  surface = undefined;
});

describe('the CodeMirror surface', () => {
  it('shows the text, reads it back, and says where the cursor is', () => {
    const editor = make('one\ntwo');

    expect(editor.text()).toBe('one\ntwo');
    expect(editor.cursor()).toEqual({ line: 1, col: 1 });
    view().dispatch({ selection: { anchor: 6 } });
    expect(editor.cursor()).toEqual({ line: 2, col: 3 });
    expect(parent.querySelectorAll('.cm-line')).toHaveLength(2);
  });

  it('reports an edit as an edit and a move of the cursor as a move; setting the text is neither', () => {
    const editor = make('one');
    const heard: boolean[] = [];
    editor.onChange((edited) => heard.push(edited));

    view().dispatch({ changes: { from: 3, insert: '!' } });
    view().dispatch({ selection: { anchor: 0 } });
    editor.setText('replaced');

    expect(heard).toEqual([true, false, false]);
    expect(editor.text()).toBe('replaced');
  });

  it('can be made read-only, and writable again', () => {
    const editor = make('one');
    editor.setReadOnly(true);

    expect(view().state.readOnly).toBe(true);
    editor.setReadOnly(false);
    expect(view().state.readOnly).toBe(false);
  });

  it('opens the search panel, with the replace field focused for a replace', () => {
    const editor = make('one');

    editor.openSearch(false);
    expect(parent.querySelector('.cm-search')).not.toBeNull();
    editor.openSearch(true);

    expect(parent.querySelector('.cm-search input[name=replace]')).toHaveFocus();
  });

  it('Ctrl+S is the save, and does not reach the page', () => {
    const onSave = vi.fn();
    make('one', 'notes.txt', onSave);

    const event = new KeyboardEvent('keydown', { key: 's', ctrlKey: true, bubbles: true, cancelable: true });
    view().contentDOM.dispatchEvent(event);

    expect(onSave).toHaveBeenCalledOnce();
    expect(event.defaultPrevented).toBe(true);
  });

  it('loads the language of the file by its name, and leaves an unknown one plain', async () => {
    make('const a = 1;', 'a.ts');
    await vi.waitFor(() => expect(parent.querySelectorAll('.cm-line span').length).toBeGreaterThan(0));
    surface?.destroy();

    make('const a = 1;', 'a.unknownextension');
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(parent.querySelectorAll('.cm-line span')).toHaveLength(0);
  });
});
