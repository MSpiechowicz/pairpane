import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { type LanguageSupport, LanguageDescription } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { openSearchPanel, search, searchKeymap } from '@codemirror/search';
import { Compartment, EditorState, type Extension } from '@codemirror/state';
import { drawSelection, EditorView, highlightActiveLine, keymap, lineNumbers } from '@codemirror/view';

import { theme } from './cmTheme';
import type { EditorSurface, SurfaceOptions } from './surface';

/** The surface of the editor, made with CodeMirror. This is the only file that names it. */
export function createSurface(options: SurfaceOptions): EditorSurface {
  const language = new Compartment();
  const readOnly = new Compartment();
  let support: LanguageSupport | undefined;
  let locked = options.readOnly;
  let listener: ((edited: boolean) => void) | undefined;
  let destroyed = false;

  const extensions: Extension[] = [
    lineNumbers(),
    history(),
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
        run: () => {
          openSearch(true);
          return true;
        },
      },
      ...searchKeymap,
      ...historyKeymap,
      ...defaultKeymap,
    ]),
    theme,
    EditorView.updateListener.of((update) => {
      if (update.docChanged || update.selectionSet) {
        listener?.(update.docChanged);
      }
    }),
  ];

  const stateFor = (doc: string) =>
    EditorState.create({
      doc,
      extensions: [...extensions, language.of(support ?? []), readOnly.of(EditorState.readOnly.of(locked))],
    });

  const view = new EditorView({ parent: options.parent, state: stateFor(options.text) });

  function openSearch(replace: boolean) {
    openSearchPanel(view);
    if (replace) {
      view.dom.querySelector<HTMLInputElement>('.cm-search input[name=replace]')?.focus();
    }
  }

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
      view.setState(stateFor(text));
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
    openSearch,
    destroy() {
      destroyed = true;
      view.destroy();
    },
  };
}
