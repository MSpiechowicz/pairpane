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

/** One side of the diff. */
export type MergeSide = 'left' | 'right';

/** One file of the diff: its name (it picks the language), its text and whether it may be changed. */
export interface MergeFile {
  name: string;
  text: string;
  readOnly: boolean;
}

export interface MergeOptions {
  parent: HTMLElement;
  left: MergeFile;
  right: MergeFile;
  /** Ctrl+S inside the text of `side`. */
  onSave: (side: MergeSide) => void;
  /** How many changed blocks the two texts have now (told at the start and after each edit). */
  onChunks: (count: number) => void;
}

/** The two texts side by side, with what differs marked. Each side is an editor of its own, with the same contract as the single one. */
export interface MergeSurface {
  left: EditorSurface;
  right: EditorSurface;
  /**
   * The texts are so long, or so costly to compare, that every edit takes a noticeable time to diff again. The merge
   * view diffs synchronously on each edit and cannot defer it, so the caller can only lighten what is drawn.
   */
  slow: boolean;
  /** Moves to the next (previous) changed block, round the end, and scrolls to it. */
  next(): void;
  previous(): void;
  /** Hides (or shows again) the long stretches that are the same in both. */
  setCollapsed(collapsed: boolean): void;
  /** The side that has the focus, or had it last. */
  focusedSide(): MergeSide;
  focus(side: MergeSide): void;
  destroy(): void;
}

export interface MergeModule {
  createMergeSurface(options: MergeOptions): MergeSurface;
}

/** The merge view, loaded on first use: the diff's code stays out of the window until the first comparison. */
export const loadMerge = (): Promise<MergeModule> => import('./cmMerge');
