import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { type LanguageSupport, LanguageDescription } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { openSearchPanel, search, searchKeymap } from '@codemirror/search';
import { Compartment, EditorState, type Extension } from '@codemirror/state';
import { drawSelection, EditorView, highlightActiveLine, highlightSpecialChars, keymap, lineNumbers } from '@codemirror/view';

import { theme } from './cmTheme';
import type { EditorSurface } from './surface';

/** What one editor of the adapter needs to know: the file's name (it picks the language) and what Ctrl+S does. */
export interface SideOptions {
  name: string;
  readOnly: boolean;
  onSave: () => void;
}

/** What a surface does with its widget that depends on who holds it: a plain editor and a side of a merge view differ in both. */
export interface Ends {
  /** Puts the text in the widget; the default is a change of the whole document that forgets the undo history. */
  replace?: (text: string) => void;
  /** Ends the widget. */
  destroy: () => void;
}

export interface Side {
  /** The extensions of an editor of this side, with the language and the read-only choice as they are now. */
  extensions(): Extension[];
  /** The surface over `view`, which was made with {@link Side.extensions}. */
  bind(view: EditorView, ends: Ends): EditorSurface;
}

/** Characters that draw as nothing or reorder the text around them (bidi controls, zero-width marks): shown as a mark, so a difference made of them is seen. */
// Bidi controls and invisible characters are shown as marks so a difference made of one is visible. The zero-width
// joiners U+200C and U+200D are left alone: emoji sequences and several scripts need them.
const INVISIBLE = /[\u061C\u200B\u200E\u200F\u202A-\u202E\u2060\u2066-\u2069\uFEFF]/;

/** The editing setup shared by the single editor and each side of the diff: keys, search, history, language, read-only. */
export function sideSetup(options: SideOptions): Side {
  const language = new Compartment();
  const readOnly = new Compartment();
  const past = new Compartment();
  let support: LanguageSupport | undefined;
  let locked = options.readOnly;
  let listener: ((edited: boolean) => void) | undefined;
  /** A replacement of the whole text is under way: it is not an edit of the user's. */
  let replacing = false;

  const base: Extension[] = [
    lineNumbers(),
    highlightSpecialChars({ addSpecialChars: INVISIBLE }),
    past.of(history()),
    drawSelection(),
    highlightActiveLine(),
    search({ top: true }),
    keymap.of([
      {
        key: 'Mod-s',
        preventDefault: true,
        run: () => {
          options.onSave();
          return true;
        },
      },
      {
        key: 'Mod-h',
        preventDefault: true,
        run: (target) => {
          openSearch(target, true);
          return true;
        },
      },
      ...searchKeymap,
      ...historyKeymap,
      ...defaultKeymap,
    ]),
    theme,
    EditorView.updateListener.of((update) => {
      if (!replacing && (update.docChanged || update.selectionSet)) {
        listener?.(update.docChanged);
      }
    }),
  ];

  function openSearch(view: EditorView, replace: boolean) {
    openSearchPanel(view);
    if (replace) {
      view.dom.querySelector<HTMLInputElement>('.cm-search input[name=replace]')?.focus();
    }
  }

  /** The whole text is replaced and the undo history starts over (the history is taken out of the state and put back). */
  function replaceAll(view: EditorView, text: string) {
    replacing = true;
    try {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: text },
        selection: { anchor: 0 },
        effects: past.reconfigure([]),
      });
      view.dispatch({ effects: past.reconfigure(history()) });
    } finally {
      replacing = false;
    }
  }

  return {
    extensions: () => [...base, language.of(support ?? []), readOnly.of(EditorState.readOnly.of(locked))],

    bind(view, ends) {
      let destroyed = false;

      // The language comes in later, by the file's name; a file the table does not know stays plain.
      void LanguageDescription.matchFilename(languages, options.name)
        ?.load()
        .then((loaded) => {
          if (!destroyed) {
            support = loaded;
            view.dispatch({ effects: language.reconfigure(loaded) });
          }
        })
        .catch(() => {});

      return {
        text: () => view.state.doc.toString(),
        setText(text) {
          (ends.replace ?? ((next) => replaceAll(view, next)))(text);
          listener?.(false);
        },
        setReadOnly(value) {
          locked = value;
          view.dispatch({ effects: readOnly.reconfigure(EditorState.readOnly.of(value)) });
        },
        onChange(next) {
          listener = next;
        },
        cursor() {
          const line = view.state.doc.lineAt(view.state.selection.main.head);

          return { line: line.number, col: view.state.selection.main.head - line.from + 1 };
        },
        focus: () => view.focus(),
        openSearch: (replace) => openSearch(view, replace),
        destroy() {
          destroyed = true;
          ends.destroy();
        },
      };
    },
  };
}
