import { describe, expect, it, vi } from 'vitest';

import { Editor } from '$lib/editor/editor.svelte';

import { answer, PICKED, world } from '../../../tests/diff';
import { editDocument } from '../../../tests/editor';

const TARGET = { path: '/dir', name: 'notes.txt' };

/** A diff and an editor on one fake backend, that know of each other as the window has them; `gate` holds back what `hold` names. */
function both({ hold, linked = true }: { hold: 'external' | 'diff'; linked?: boolean }) {
  let release: () => void = () => {};
  const gate = new Promise<void>((resolve) => (release = resolve));
  let editor: Editor | undefined;
  const opened = world(
    {},
    {
      edit_external: (async () => {
        if (hold === 'external') {
          await gate;
        }

        return 'notConfigured';
      }) as never,
      edit_open: (async (args: { name: string }) => {
        if (hold === 'diff' && args.name !== TARGET.name) {
          await gate;
        }

        return { kind: 'text', document: editDocument({ name: args.name, text: `text of ${args.name}\n`, revision: `r-${args.name}` }) };
      }) as never,
    },
    linked ? { blocked: () => editor?.busy === true } : {},
  );
  const hooks = { under: () => TARGET, say: opened.say, view: vi.fn(), ...(linked ? { blocked: () => opened.diff.busy } : {}) };
  editor = new Editor(opened.backend.ipc, hooks, opened.unsaved);

  return { ...opened, editor, release };
}

describe('one overlay at a time', () => {
  it('does not open the editor while a comparison is being opened', async () => {
    const { diff, editor, sent, release } = both({ hold: 'diff' });

    const opening = diff.openPicked(PICKED);
    await editor.open(TARGET, { builtin: false });
    release();
    await opening;

    expect(editor.modal).toBe(false);
    expect(sent('edit_external')).toEqual([]);
    expect(diff.mode).toBe('text');
  });

  it('does not open a comparison while the editor is being opened', async () => {
    const { diff, editor, sent, release } = both({ hold: 'external' });

    const opening = editor.open(TARGET, { builtin: false });
    await diff.openPicked(PICKED);
    release();
    await opening;

    expect(diff.modal).toBe(false);
    expect(sent('edit_open').map(({ name }) => name)).toEqual(['notes.txt']);
    expect(editor.modal).toBe(true);
  });
});

describe('a window close with changes in both overlays', () => {
  /** Both are open and changed, which the window itself no longer allows. */
  async function changedBoth() {
    const world_ = both({ hold: 'external', linked: false });
    world_.release();
    await world_.editor.open(TARGET, { builtin: true });
    await world_.diff.openPicked(PICKED);
    world_.editor.session?.setLineEnding('crlf');
    world_.diff.pair?.right.setLineEnding('crlf');

    return world_;
  }

  it('asks about each, and lets the window go once both are dealt with', async () => {
    const { diff, editor, sent } = await changedBoth();

    const closing = editor.onCloseRequested(() => diff.discard());
    await vi.waitFor(() => expect(editor.question?.title).toBe('Save changes to notes.txt?'));
    answer(editor, 'alternate');
    await vi.waitFor(() => expect(diff.question?.title).toBe('Save changes to b.txt?'));
    expect(sent('app_close_confirmed')).toEqual([]);
    answer(diff, 'alternate');
    await closing;

    expect(sent('app_close_confirmed')).toHaveLength(1);
  });

  it('a Cancel on the first keeps the window, and does not ask about the second', async () => {
    const { diff, editor, sent } = await changedBoth();

    const closing = editor.onCloseRequested(() => diff.discard());
    await vi.waitFor(() => expect(editor.question).toBeDefined());
    answer(editor, 'cancel');
    await closing;

    expect(diff.question).toBeUndefined();
    expect(sent('app_close_confirmed')).toEqual([]);
  });
});
