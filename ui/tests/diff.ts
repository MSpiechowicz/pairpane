// Test helpers for the file diff: a merge surface made of two plain textareas (CodeMirror is not what these tests
// are about), and the answers of a fake backend for two files that open as text.
import type { EditDocument } from '$lib/ipc/bindings/EditDocument';
import type { MergeOptions, MergeSide, MergeSurface } from '$lib/editor/surface';

import type { Answers } from './app';
import { editAnswers, editDocument, FakeSurface } from './editor';

/** Every merge surface made, the last one is the one on screen. */
export const merges: FakeMerge[] = [];

/** Two textareas for the two sides; the changed blocks are the lines that differ. */
export class FakeMerge implements MergeSurface {
  readonly left: FakeSurface;
  readonly right: FakeSurface;
  /** What was asked of it, in order: `next`, `previous`, `collapse`, `expand`, `focus left`… */
  readonly calls: string[] = [];
  #last: MergeSide = 'left';

  constructor(options: MergeOptions) {
    const side = (name: MergeSide) => {
      const file = options[name];
      const surface = new FakeSurface({ parent: options.parent, name: file.name, text: file.text, readOnly: file.readOnly, onSave: () => {} });
      surface.area.addEventListener('focus', () => (this.#last = name));
      surface.area.addEventListener('input', () => options.onChunks(this.#changes()));
      surface.area.addEventListener('keydown', (event) => {
        if (event.key.toLowerCase() === 's' && event.ctrlKey) {
          event.preventDefault();
          options.onSave(name);
        }
      });

      return surface;
    };

    this.left = side('left');
    this.right = side('right');
    options.onChunks(this.#changes());
    merges.push(this);
  }

  #changes(): number {
    const [left, right] = [this.left.text().split('\n'), this.right.text().split('\n')];
    let count = 0;
    for (let line = 0; line < Math.max(left.length, right.length); line += 1) {
      if (left[line] !== right[line]) {
        count += 1;
      }
    }

    return count;
  }

  next = () => void this.calls.push('next');
  previous = () => void this.calls.push('previous');
  setCollapsed = (collapsed: boolean) => void this.calls.push(collapsed ? 'collapse' : 'expand');
  focusedSide = () => this.#last;
  focus(side: MergeSide) {
    this.calls.push(`focus ${side}`);
    (side === 'left' ? this.left : this.right).focus();
  }
  destroy() {
    this.left.destroy();
    this.right.destroy();
  }
}

/** The module `$lib/editor/cmMerge` is replaced with: `vi.mock('$lib/editor/cmMerge', () => import('../tests/diff'))`. */
export const createMergeSurface = (options: MergeOptions): MergeSurface => new FakeMerge(options);

/**
 * The answers of a fake backend for the diff: every file opens as text, as `documents` says for its name (`text`,
 * a revision made of its name); `extra` replaces any answer.
 */
export function diffAnswers(documents: Record<string, Partial<EditDocument>> = {}, extra: Answers = {}): Answers {
  return editAnswers(
    {},
    {
      edit_open: (args: { name: string; encoding: string | null }) => ({
        kind: 'text',
        document: editDocument({ name: args.name, text: `text of ${args.name}\n`, revision: `r-${args.name}`, ...documents[args.name] }),
      }),
      ...extra,
    },
  );
}
