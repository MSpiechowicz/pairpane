import { EditorView } from '@codemirror/view';
import { goToNextChunk, goToPreviousChunk, MergeView } from '@codemirror/merge';

import { sideSetup } from './cmSetup';
import type { MergeOptions, MergeSide, MergeSurface } from './surface';

/** Long stretches of the same lines fold away, three lines of them stay around each change. */
const COLLAPSE = { margin: 3, minSize: 4 };

/**
 * Two long files that differ everywhere give up the fine diff (and show whole lines changed) rather than freeze the
 * window. The merge view halves the scan limit; above about 4000 a repetitive text (one letter, a data file of
 * equal records) takes minutes, as the fallback for a large changed region never looks at the timeout. At 1000 the
 * worst such pairs of 100 KiB to 2 MiB build in tens of milliseconds.
 */
const DIFF = { scanLimit: 1000, timeout: 500 };

/** A comparison of texts this long (a side), or one that took this long to build, starts with the unchanged lines folded: every edit diffs again at about that cost. */
const SLOW = { chars: 2 * 1024 * 1024, buildMs: 300 };

/** The marks of a change, in the app's colours (the merge view's own are fixed light and dark ones). */
const marks = EditorView.theme({
  '&.cm-merge-a .cm-changedLine, .cm-deletedChunk': { backgroundColor: 'color-mix(in srgb, var(--danger) 10%, transparent)' },
  '&.cm-merge-b .cm-changedLine, .cm-inlineChangedLine': { backgroundColor: 'color-mix(in srgb, var(--ok) 10%, transparent)' },
  '.cm-mergeView &.cm-merge-a .cm-changedText, .cm-mergeView .cm-deletedChunk .cm-deletedText': {
    background: 'color-mix(in srgb, var(--danger) 35%, transparent)',
  },
  '.cm-mergeView &.cm-merge-b .cm-changedText': { background: 'color-mix(in srgb, var(--ok) 35%, transparent)' },
  '.cm-mergeView &.cm-merge-a .cm-changedLineGutter, .cm-mergeView .cm-deletedLineGutter': { background: 'var(--danger)' },
  '.cm-mergeView &.cm-merge-b .cm-changedLineGutter': { background: 'var(--ok)' },
  '.cm-collapsedLines': { color: 'var(--dim)', background: 'var(--panel-2)' },
});

/** The merge surface, made with CodeMirror's merge view: each side is an editor of the same make as the single one. */
export function createMergeSurface(options: MergeOptions): MergeSurface {
  const setups = {
    left: sideSetup({ ...options.left, onSave: () => options.onSave('left') }),
    right: sideSetup({ ...options.right, onSave: () => options.onSave('right') }),
  };
  let merge: MergeView | undefined;
  let last: MergeSide = 'left';

  /** What each side tells the surface: that it was focused, and the new count of changed blocks after an edit. */
  const watch = (side: MergeSide) =>
    EditorView.updateListener.of((update) => {
      if (update.focusChanged && update.view.hasFocus) {
        last = side;
      }

      if (update.docChanged && merge !== undefined) {
        options.onChunks(merge.chunks.length);
      }
    });

  const started = performance.now();
  const view = new MergeView({
    parent: options.parent,
    a: { doc: options.left.text, extensions: [...setups.left.extensions(), marks, watch('left')] },
    b: { doc: options.right.text, extensions: [...setups.right.extensions(), marks, watch('right')] },
    highlightChanges: true,
    gutter: true,
    diffConfig: DIFF,
  });
  const slow = Math.max(options.left.text.length, options.right.text.length) > SLOW.chars || performance.now() - started > SLOW.buildMs;
  merge = view;
  options.onChunks(view.chunks.length);

  // The two sides are destroyed with the merge view, not each on its own.
  const ends = { destroy: () => {} };
  const left = setups.left.bind(view.a, ends);
  const right = setups.right.bind(view.b, ends);
  const editors = { left: view.a, right: view.b };

  const focused = (): MergeSide => {
    if (view.a.hasFocus) {
      return 'left';
    }

    return view.b.hasFocus ? 'right' : last;
  };

  return {
    left,
    right,
    slow,
    next: () => void goToNextChunk(editors[focused()]),
    previous: () => void goToPreviousChunk(editors[focused()]),
    setCollapsed: (collapsed) => view.reconfigure({ collapseUnchanged: collapsed ? COLLAPSE : undefined }),
    focusedSide: focused,
    focus: (side) => editors[side].focus(),
    destroy() {
      left.destroy();
      right.destroy();
      view.destroy();
    },
  };
}
