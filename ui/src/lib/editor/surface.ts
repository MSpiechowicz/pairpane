/** What the editor overlay draws the text with: the editing widget itself, behind an interface the tests can replace. */
export interface EditorSurface {
  /** The whole text, with every line break as `\n`. */
  text(): string;
  /** Replaces the text and forgets the undo history; it does not report a change. */
  setText(text: string): void;
  setReadOnly(readOnly: boolean): void;
  /** Hears of every change of the text (`edited`) and of every move of the cursor (not `edited`). */
  onChange(listener: (edited: boolean) => void): void;
  /** Where the cursor is, counted from 1. */
  cursor(): { line: number; col: number };
  focus(): void;
  /** Opens the search panel, with the focus on its replace field when `replace` is set. */
  openSearch(replace: boolean): void;
  destroy(): void;
}

export interface SurfaceOptions {
  parent: HTMLElement;
  /** The file's name: it picks the language. */
  name: string;
  text: string;
  readOnly: boolean;
  /** Ctrl+S inside the text. */
  onSave: () => void;
}

export interface SurfaceModule {
  createSurface(options: SurfaceOptions): EditorSurface;
}

/** The CodeMirror surface, loaded on first use: the editor's code stays out of the window until F4 needs it. */
export const loadSurface = (): Promise<SurfaceModule> => import('./cm');
