import { fireEvent, screen, within } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import type { RenamePreview } from '$lib/ipc/bindings/RenamePreview';
import { IpcFailure } from '$lib/ipc/transport';

import { type Answers, appShown, key, listbox, pane } from '../tests/app';
import { job } from '../tests/ops';

/** Every name gets `new-` in front of it, as the backend would say. */
const prefixed = ({ names }: { names: string[] }): RenamePreview => ({
  kind: 'rows',
  rows: names.map((name) => ({ from: name, to: `new-${name}`, problem: null })),
  changed: names.length,
  problems: 0,
});

const shown = (answers: Answers = {}) => appShown(invoke, { rename_preview: prefixed as never, ops_task: () => 5, ...answers });
const apply = () => screen.getByRole('button', { name: 'Rename' });
const notice = () => screen.findByRole('region', { name: 'Notice' });

/** A rename job as the backend adds it, and as it ends. */
const added = (id: number, items: number) => ({ kind: 'added' as const, job: job(id, { kind: 'rename', phase: 'renaming', items }) });
const ended = (id: number, skipped = 0) => ({
  kind: 'ended' as const,
  id,
  outcome: { kind: 'done' as const },
  skipped,
  errors: [],
  moreErrors: 0,
});

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('Multi-rename', () => {
  it('Ctrl+M opens the dialog on the item under the cursor, and Escape gives the list the focus back', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'm', { ctrlKey: true });

    expect(await screen.findByRole('dialog', { name: 'Rename “a.pdf”' })).toBeInTheDocument();
    await vi.waitFor(() => expect(backend.sent('rename_preview')).toHaveLength(1));
    expect(backend.sent('rename_preview')[0]).toMatchObject({ path: '/left', names: ['a.pdf'] });

    await key(screen.getByRole('textbox', { name: 'Name' }), 'Escape');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('the Files menu item opens it on the selection, which the job takes when it starts', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'Insert');
    await key(listbox('Left'), 'Insert');

    await fireEvent.click(screen.getByRole('button', { name: 'Files' }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Multi-rename…' }));

    expect(await screen.findByRole('dialog', { name: 'Rename 2 items' })).toBeInTheDocument();
    await vi.waitFor(() => expect(apply()).toBeEnabled());
    await fireEvent.click(apply());

    await vi.waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(backend.sent('ops_task')).toEqual([
      { from: '/left', names: ['a.pdf', 'b.txt'], task: { kind: 'rename', targets: ['new-a.pdf', 'new-b.txt'] } },
    ]);
    expect(within(pane('Left pane')).queryByText(/^Selected:/)).not.toBeInTheDocument();
  });

  it('is nothing on the `..` row', async () => {
    await shown();
    await key(listbox('Left'), 'Home');

    await key(listbox('Left'), 'm', { ctrlKey: true });

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('a dialog that is open keeps the other shortcuts to itself', async () => {
    await shown();
    await key(listbox('Left'), 'm', { ctrlKey: true });
    await screen.findByRole('dialog');

    await key(window, 'F8');

    expect(screen.getAllByRole('dialog')).toHaveLength(1);
    expect(screen.queryByRole('dialog', { name: /Trash/ })).not.toBeInTheDocument();
  });
});

describe('after a rename', () => {
  async function renamed() {
    const backend = await shown({ ops_rename_undo: () => 6 });
    await key(listbox('Left'), 'm', { ctrlKey: true });
    await vi.waitFor(() => expect(apply()).toBeEnabled());
    await fireEvent.click(apply());
    await vi.waitFor(() => expect(backend.sent('ops_task')).toHaveLength(1));
    return backend;
  }

  it('says how many items were renamed and offers Undo, which queues the reverse job and says how that went', async () => {
    const backend = await renamed();
    backend.ops(added(5, 1), ended(5));

    expect(await notice()).toHaveTextContent('Renamed 1 item.');
    await fireEvent.click(screen.getByRole('button', { name: 'Undo' }));

    expect(backend.sent('ops_rename_undo')).toEqual([{ job: 5 }]);
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();

    backend.ops(added(6, 3), ended(6, 1));

    expect(await screen.findByText('Restored 2, skipped 1.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();
  });

  it('an undo that is refused as still running stays offered; one with nothing to undo does not', async () => {
    let answer = () => {
      throw new IpcFailure({ code: 'invalidInput', message: 'The rename is still running.' });
    };
    const backend = await renamed();
    invoke.mockImplementation(async (name: string) => {
      if (name === 'ops_rename_undo') {
        answer();
      }

      return null;
    });
    backend.ops(added(5, 1), ended(5));
    await notice();

    await fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    expect(await screen.findByText('The rename is still running.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Undo' })).toBeInTheDocument();

    answer = () => {
      throw new IpcFailure({ code: 'notFound', message: 'Nothing to undo.' });
    };
    await fireEvent.click(screen.getByRole('button', { name: 'Undo' }));

    expect(await screen.findByText('Nothing to undo.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();
  });

  it('other jobs ending say nothing, and a Trash undo still goes to the Trash', async () => {
    const backend = await shown({ ops_trash: () => ({ undo: 4, failure: null }), ops_untrash: () => 1 });
    backend.ops({ kind: 'added', job: job(1, { kind: 'copy' }) }, ended(1));
    expect(screen.queryByRole('region', { name: 'Notice' })).not.toBeInTheDocument();

    await key(listbox('Left'), 'F8');
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Undo' }));

    expect(backend.sent('ops_untrash')).toEqual([{ token: 4 }]);
    expect(backend.sent('ops_rename_undo')).toEqual([]);
    expect(await screen.findByText('Restored 1 item.')).toBeInTheDocument();
  });
});
