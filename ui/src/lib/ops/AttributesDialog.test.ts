import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Entry } from '$lib/ipc/bindings/Entry';
import { IpcFailure } from '$lib/ipc/transport';

import { opsBackend } from '../../../tests/ops';
import { entry } from '../../../tests/panes';
import AttributesDialog from './AttributesDialog.svelte';

const WINDOWS = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)';

function open(entries: Entry[] = [entry('a.pdf', 'file', { mode: 0o644 })], answers: Parameters<typeof opsBackend>[0] = {}) {
  const backend = opsBackend(answers);
  const onClose = vi.fn();
  const onDone = vi.fn();
  render(AttributesDialog, { ipc: backend.ipc, path: '/work', entries, onClose, onDone });
  return { ...backend, onClose, onDone };
}

const box = (name: string) => screen.getByRole('checkbox', { name });
const apply = () => fireEvent.click(screen.getByRole('button', { name: 'Apply' }));

/** The change that `ops_task` was asked for. */
const asked = (calls: (name: 'ops_task') => unknown[]) => (calls('ops_task')[0] as { task: { change: unknown } }).task.change;

afterEach(() => vi.restoreAllMocks());

describe('AttributesDialog on Unix', () => {
  it('shows the permissions the item has, as a grid of read, write and execute', () => {
    open([entry('a.pdf', 'file', { mode: 0o4640 })]);

    expect(screen.getByRole('dialog', { name: 'Change attributes of “a.pdf”' })).toBeInTheDocument();
    expect(box('Owner read')).toBeChecked();
    expect(box('Owner write')).toBeChecked();
    expect(box('Owner execute')).not.toBeChecked();
    expect(box('Group read')).toBeChecked();
    expect(box('Group write')).not.toBeChecked();
    expect(box('Others read')).not.toBeChecked();
    expect(box('Set user ID')).toBeChecked();
    expect(box('Sticky')).not.toBeChecked();
    expect(screen.queryByRole('checkbox', { name: 'Read-only' })).not.toBeInTheDocument();
  });

  it('a box the items agree on is set or cleared by a click, nothing in between', async () => {
    open();
    const execute = box('Owner execute');

    await fireEvent.click(execute);
    expect(execute).toBeChecked();
    await fireEvent.click(execute);
    expect(execute).not.toBeChecked();
    expect(execute).not.toBePartiallyChecked();
  });

  it('shows a dash where the items differ, and a click goes round: set, clear, leave alone', async () => {
    open([entry('a', 'file', { mode: 0o755 }), entry('b', 'file', { mode: 0o644 })]);
    const execute = box('Owner execute');

    expect(execute).toBePartiallyChecked();
    await fireEvent.click(execute);
    expect(execute).toBeChecked();
    await fireEvent.click(execute);
    expect(execute).not.toBeChecked();
    expect(execute).not.toBePartiallyChecked();
    await fireEvent.click(execute);
    expect(execute).toBePartiallyChecked();
    expect(screen.getByRole('dialog', { name: 'Change attributes of 2 items' })).toBeInTheDocument();
  });

  it('asks for the permissions that were changed, and nothing else', async () => {
    const { calls, onDone } = open();

    await fireEvent.click(box('Owner execute'));
    await fireEvent.click(box('Group write'));
    await fireEvent.click(box('Group write'));
    await fireEvent.click(box('Others read'));
    await apply();

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(calls('ops_task')).toEqual([
      {
        from: '/work',
        names: ['a.pdf'],
        task: {
          kind: 'attributes',
          change: {
            mode: { kind: 'adjust', add: 0o100, remove: 0o004 },
            readOnly: null,
            modified: null,
            accessed: null,
            recursive: false,
          },
        },
      },
    ]);
  });

  it('asks for nothing when nothing was changed, and says so', async () => {
    const { invoke, onDone } = open();

    await apply();

    expect(screen.getByRole('alert')).toHaveTextContent('Choose what to change.');
    expect(invoke).not.toHaveBeenCalled();
    expect(onDone).not.toHaveBeenCalled();
  });

  it('typing a time chooses to set it; the time goes as milliseconds', async () => {
    const { calls } = open();
    const modified = screen.getByRole('textbox', { name: 'Modified time' });
    expect(box('Modified')).not.toBeChecked();

    await fireEvent.input(modified, { target: { value: '2026-01-02 03:04:05' } });
    expect(box('Modified')).toBeChecked();
    await fireEvent.click(box('Accessed'));
    await fireEvent.input(screen.getByRole('textbox', { name: 'Accessed time' }), { target: { value: '2025-12-31 23:59' } });
    await apply();

    await vi.waitFor(() => expect(calls('ops_task')).toHaveLength(1));
    expect(asked(calls)).toEqual({
      mode: null,
      readOnly: null,
      modified: new Date(2026, 0, 2, 3, 4, 5).getTime(),
      accessed: new Date(2025, 11, 31, 23, 59, 0).getTime(),
      recursive: false,
    });
  });

  it('starts the time field with the item`s own modification time', () => {
    open([entry('a.pdf', 'file', { mode: 0o644, modified: new Date(2026, 10, 20, 7, 58, 12).getTime() })]);

    expect(screen.getByRole('textbox', { name: 'Modified time' })).toHaveValue('2026-11-20 07:58:12');
  });

  it('a time that is none is not sent', async () => {
    const { invoke } = open();
    await fireEvent.input(screen.getByRole('textbox', { name: 'Modified time' }), { target: { value: 'tomorrow' } });

    await apply();

    expect(screen.getByRole('alert')).toHaveTextContent(/^Type a time like \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.$/);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('offers to go into folders only when a folder is among the items', async () => {
    open();
    expect(screen.queryByRole('checkbox', { name: 'Include what is inside folders' })).not.toBeInTheDocument();
  });

  it('a change that goes into folders sends every permission that is set or cleared, not only the changed ones', async () => {
    const { calls } = open([entry('docs', 'dir', { mode: 0o755 })]);

    await fireEvent.click(box('Include what is inside folders'));
    await fireEvent.click(box('Group write'));
    await apply();

    await vi.waitFor(() => expect(calls('ops_task')).toHaveLength(1));
    expect(asked(calls)).toMatchObject({
      mode: { kind: 'adjust', add: 0o775, remove: 0o4000 | 0o2000 | 0o1000 | 0o002 },
      recursive: true,
    });
  });

  it('a folder without the box checked does not go into folders', async () => {
    const { calls } = open([entry('docs', 'dir', { mode: 0o755 })]);

    await fireEvent.click(box('Group write'));
    await apply();

    await vi.waitFor(() => expect(calls('ops_task')).toHaveLength(1));
    expect(asked(calls)).toMatchObject({ mode: { kind: 'adjust', add: 0o020, remove: 0 }, recursive: false });
  });

  it('Enter in a box asks, Escape and Cancel leave', async () => {
    const { calls, onClose } = open();

    await fireEvent.click(box('Owner execute'));
    await fireEvent.keyDown(box('Owner execute'), { key: 'Enter' });
    await vi.waitFor(() => expect(calls('ops_task')).toHaveLength(1));

    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('a refusal is shown and the dialog stays', async () => {
    const { onDone } = open(undefined, {
      ops_task: async () => {
        throw new IpcFailure({ code: 'unsupported', message: 'this system has no Unix permissions to set' });
      },
    });
    await fireEvent.click(box('Owner execute'));

    await apply();

    expect(await screen.findByRole('alert')).toHaveTextContent('this system has no Unix permissions to set');
    expect(onDone).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Apply' })).toBeEnabled();
  });
});

describe('AttributesDialog on Windows', () => {
  const windows = () => vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(WINDOWS);

  it('has the read-only attribute and no permissions', () => {
    windows();
    open([entry('a.pdf', 'file', { readonly: true })]);

    expect(box('Read-only')).toBeChecked();
    expect(screen.queryByRole('checkbox', { name: 'Owner read' })).not.toBeInTheDocument();
    expect(screen.queryByRole('group', { name: 'Permissions' })).not.toBeInTheDocument();
  });

  it('asks to set or clear read-only, never permissions', async () => {
    windows();
    const { calls } = open([entry('a.pdf'), entry('b.txt')]);
    expect(box('Read-only')).not.toBeChecked();

    await fireEvent.click(box('Read-only'));
    await apply();

    await vi.waitFor(() => expect(calls('ops_task')).toHaveLength(1));
    expect(asked(calls)).toEqual({ mode: null, readOnly: true, modified: null, accessed: null, recursive: false });
  });

  it('shows a dash when the items differ, and sends nothing for it', async () => {
    windows();
    const { invoke } = open([entry('a.pdf', 'file', { readonly: true }), entry('b.txt')]);

    expect(box('Read-only')).toBePartiallyChecked();
    await apply();

    expect(screen.getByRole('alert')).toHaveTextContent('Choose what to change.');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('can set a time on its own', async () => {
    windows();
    const { calls } = open([entry('a.pdf')]);

    await fireEvent.input(screen.getByRole('textbox', { name: 'Modified time' }), { target: { value: '2026-01-02 03:04:05' } });
    await apply();

    await vi.waitFor(() => expect(calls('ops_task')).toHaveLength(1));
    expect(asked(calls)).toMatchObject({ readOnly: null, modified: new Date(2026, 0, 2, 3, 4, 5).getTime() });
  });
});
