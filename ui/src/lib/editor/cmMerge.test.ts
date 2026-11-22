import { EditorView } from '@codemirror/view';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { createMergeSurface } from './cmMerge';
import type { MergeFile, MergeSide, MergeSurface } from './surface';

const parent = document.body.appendChild(document.createElement('div'));
let surface: MergeSurface | undefined;

const file = (text: string, extra: Partial<MergeFile> = {}): MergeFile => ({ name: 'notes.txt', text, readOnly: false, ...extra });

function make(left: MergeFile, right: MergeFile) {
  const saves: MergeSide[] = [];
  const counts: number[] = [];
  surface = createMergeSurface({ parent, left, right, onSave: (side) => saves.push(side), onChunks: (count) => counts.push(count) });

  return { surface, saves, counts };
}

/** The editor of a side. */
const editor = (side: MergeSide) => EditorView.findFromDOM(parent.querySelector<HTMLElement>(side === 'left' ? '.cm-merge-a' : '.cm-merge-b')!)!;

/** A data file of identical records, a few of them changed: what a line diff of a long repetitive stretch is worst at. */
const records = (count: number, changed: boolean) =>
  Array.from({ length: count }, (_, index) => (changed && index % 211 === 0 ? '0,0,0,1,0,0,0,0' : '0,0,0,0,0,0,0,0')).join('\n');

const lines = (count: number, change: Record<number, string> = {}) =>
  Array.from({ length: count }, (_, index) => change[index] ?? `line ${index}`).join('\n');

afterEach(() => {
  surface?.destroy();
  surface = undefined;
  parent.textContent = '';
});

describe('the merge surface', () => {
  it('shows both texts and counts the changed blocks, again after an edit of either side', () => {
    const { counts } = make(file('a\nb\nc\nd'), file('a\nB\nc\nd'));

    expect(surface?.left.text()).toBe('a\nb\nc\nd');
    expect(surface?.right.text()).toBe('a\nB\nc\nd');
    expect(counts).toEqual([1]);

    editor('right').dispatch({ changes: { from: 6, to: 7, insert: 'D' } });
    expect(counts.at(-1)).toBe(2);

    editor('left').dispatch({ changes: { from: 6, to: 7, insert: 'D' } });
    expect(counts.at(-1)).toBe(1);
  });

  it('counts nothing for two equal texts', () => {
    const { counts } = make(file('same'), file('same'));

    expect(counts).toEqual([0]);
  });

  it('hears an edit of each side on its own surface, and a replaced text is not an edit', () => {
    const { surface: both } = make(file('one'), file('one'));
    const heard: Record<MergeSide, boolean[]> = { left: [], right: [] };
    both.left.onChange((edited) => heard.left.push(edited));
    both.right.onChange((edited) => heard.right.push(edited));

    editor('right').dispatch({ changes: { from: 3, insert: '!' } });
    both.left.setText('replaced');

    expect(heard.left).toEqual([false]);
    expect(heard.right).toEqual([true]);
    expect(both.left.text()).toBe('replaced');
    expect(both.right.text()).toBe('one!');
  });

  it('forgets the undo history of a side whose text was replaced', () => {
    const { surface: both } = make(file('one'), file('one'));
    editor('left').dispatch({ changes: { from: 3, insert: '!' } });

    both.left.setText('fresh');
    editor('left').contentDOM.dispatchEvent(new KeyboardEvent('keydown', { key: 'z', ctrlKey: true, bubbles: true, cancelable: true }));

    expect(both.left.text()).toBe('fresh');
  });

  it('moves to the next and the previous changed block of the side that was focused last (the left one at first), round the ends', () => {
    const { surface: both } = make(file(lines(40, { 5: 'x', 20: 'y' })), file(lines(40, { 5: 'X', 20: 'Y' })));
    const where = () => editor('left').state.doc.lineAt(editor('left').state.selection.main.head).number;

    both.next();
    expect(where()).toBe(6);
    both.next();
    expect(where()).toBe(21);
    both.next();
    expect(where()).toBe(6);
    both.previous();
    expect(where()).toBe(21);
  });

  it('can have a side read-only, and writable again', () => {
    make(file('one'), file('one', { readOnly: true }));

    expect(editor('left').state.readOnly).toBe(false);
    expect(editor('right').state.readOnly).toBe(true);
    surface?.right.setReadOnly(false);
    expect(editor('right').state.readOnly).toBe(false);
  });

  it('Ctrl+S in a side saves that side, and does not reach the page', () => {
    const { saves } = make(file('one'), file('two'));

    const event = new KeyboardEvent('keydown', { key: 's', ctrlKey: true, bubbles: true, cancelable: true });
    editor('right').contentDOM.dispatchEvent(event);
    editor('left').contentDOM.dispatchEvent(new KeyboardEvent('keydown', { key: 's', ctrlKey: true, bubbles: true, cancelable: true }));

    expect(saves).toEqual(['right', 'left']);
    expect(event.defaultPrevented).toBe(true);
  });

  it('says which side has the focus, and keeps saying it after the focus is gone', () => {
    const { surface: both } = make(file('one'), file('two'));

    both.focus('right');
    expect(both.focusedSide()).toBe('right');
    both.focus('left');
    expect(both.focusedSide()).toBe('left');
    (document.activeElement as HTMLElement).blur();
    expect(both.focusedSide()).toBe('left');
  });

  it('folds the unchanged stretches away on request, and shows them again', () => {
    const { surface: both } = make(file(lines(60, { 30: 'x' })), file(lines(60, { 30: 'X' })));

    expect(parent.querySelectorAll('.cm-collapsedLines')).toHaveLength(0);
    both.setCollapsed(true);
    expect(parent.querySelectorAll('.cm-collapsedLines').length).toBeGreaterThan(0);
    both.setCollapsed(false);
    expect(parent.querySelectorAll('.cm-collapsedLines')).toHaveLength(0);
  });

  it('marks the changed lines and the changed words', async () => {
    make(file('keep\nthe old words'), file('keep\nthe new words'));

    await vi.waitFor(() => expect(parent.querySelectorAll('.cm-changedLine')).toHaveLength(2));
    expect(parent.querySelectorAll('.cm-changedText').length).toBeGreaterThan(0);
  });

  it('compares a long repetitive stretch at once (the fine diff is given up, not waited for)', () => {
    const started = performance.now();
    const { counts } = make(file(records(12_000, false)), file(records(12_000, true)));

    expect(performance.now() - started).toBeLessThan(1000);
    expect(counts[0]).toBeGreaterThan(0);
  });

  it('starts a long comparison folded, and a short one not', () => {
    const { surface: short } = make(file(lines(60, { 30: 'x' })), file(lines(60, { 30: 'X' })));
    expect(short.slow).toBe(false);
    short.destroy();

    const long = createMergeSurface({
      parent,
      left: file('x\n'.repeat(1_100_000)),
      right: file('x\n'.repeat(1_100_000)),
      onSave: () => {},
      onChunks: () => {},
    });
    expect(long.slow).toBe(true);
    long.destroy();
  });

  it('draws the characters that are invisible or turn the text around as marks, in each side', () => {
    make(file('a\u202Ab\u2060c'), file('plain'));

    for (const side of ['left', 'right'] as const) {
      expect(editor(side).dom.querySelectorAll('.cm-specialChar')).toHaveLength(side === 'left' ? 2 : 0);
    }
  });
});
