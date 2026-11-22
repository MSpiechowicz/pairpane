// Test helpers for the file diff: a merge surface made of two plain textareas (CodeMirror is not what these tests
// are about), and the answers of a fake backend for two files that open as text.
import { vi } from 'vitest';

import type { Question } from '$lib/editor/saveFlow.svelte';
import { Unsaved } from '$lib/editor/unsaved';
import type { EditDocument } from '$lib/ipc/bindings/EditDocument';
import type { EditOpened } from '$lib/ipc/bindings/EditOpened';
import type { EditSaveOptions } from '$lib/ipc/bindings/EditSaveOptions';
import type { EditSaved } from '$lib/ipc/bindings/EditSaved';
import type { ViewKind } from '$lib/ipc/bindings/ViewKind';
import { type DiffHooks, FileDiff } from '$lib/diff/fileDiff.svelte';
import type { Picked } from '$lib/diff/pick';
import type { MergeOptions, MergeSide, MergeSurface } from '$lib/editor/surface';

import type { Answers } from './app';
import { editAnswers, editDocument, FakeSurface } from './editor';
import { fakeBackend } from './panes';

/** Every merge surface made, the last one is the one on screen. */
export const merges: FakeMerge[] = [];

/** What the next merge surface made says of itself: set `slow` for a comparison that is costly to diff. */
export const next = { slow: false };

/** Two textareas for the two sides; the changed blocks are the lines that differ. */
export class FakeMerge implements MergeSurface {
  readonly left: FakeSurface;
  readonly right: FakeSurface;
  /** What was asked of it, in order: `next`, `previous`, `collapse`, `expand`, `focus left`… */
  readonly calls: string[] = [];
  readonly slow = next.slow;
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

export const PICKED: Picked = { left: { path: '/one', name: 'a.txt' }, right: { path: '/two', name: 'b.txt' } };

type BackendAnswers = Parameters<typeof fakeBackend>[0];

/** A diff on a fake backend: `a.txt` and `b.txt` open as text unless `opened` says what a name opens as. */
export function world(opened: Record<string, EditOpened> = {}, answers: BackendAnswers = {}, hooks: Partial<DiffHooks> = {}) {
  let session = 0;
  const kinds: Record<string, ViewKind> = {};
  const backend = fakeBackend({
    edit_open: ((args: { name: string; encoding: string | null }) =>
      opened[args.name] ?? {
        kind: 'text',
        document: editDocument({ name: args.name, text: `text of ${args.name}\n`, revision: `r-${args.name}`, encoding: args.encoding ?? 'UTF-8' }),
      }) as never,
    edit_save: (() => ({ kind: 'saved', size: 1, revision: 'next' }) satisfies EditSaved) as never,
    view_open: ((args: { name: string }) => {
      session += 1;
      return { session, name: args.name, size: 10, modified: null, kind: kinds[args.name] ?? { kind: 'binary' }, encoding: '' };
    }) as never,
    ...answers,
  });
  const say = vi.fn();
  const pick = vi.fn((): Picked | string => PICKED);
  const unsaved = new Unsaved(backend.ipc);
  const diff = new FileDiff(backend.ipc, unsaved, { pick, say, ...hooks });
  const sent = (name: string) => backend.invoke.mock.calls.filter(([command]) => command === name).map(([, args]) => args as never as Record<string, unknown>);
  const saves = () => sent('edit_save').map((args) => ({ name: args.name, text: args.text, options: args.options as EditSaveOptions }));

  return { backend, diff, unsaved, say, pick, sent, saves };
}

/** The question on the screen, answered. */
export const answer = (diff: { question: Question | undefined }, choice: 'confirm' | 'alternate' | 'cancel') => diff.question?.answer(choice);
