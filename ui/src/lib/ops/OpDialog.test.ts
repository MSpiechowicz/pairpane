import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import { IpcFailure } from '$lib/ipc/transport';

import { opsBackend } from '../../../tests/ops';
import OpDialog from './OpDialog.svelte';
import type { OpRequest } from './targets';

const copy = (names: string[], to = '/backup'): OpRequest => ({ kind: 'copy', from: '/work', names, to });

function open(request: OpRequest, answers: Parameters<typeof opsBackend>[0] = {}) {
  const backend = opsBackend(answers);
  const onClose = vi.fn();
  const onDone = vi.fn();
  render(OpDialog, { request, ipc: backend.ipc, onClose, onDone });
  return { ...backend, onClose, onDone, field: screen.getByRole('textbox') as HTMLInputElement };
}

describe('OpDialog copy and move', () => {
  it('names one item, or counts several', () => {
    open(copy(['a.pdf']));
    expect(screen.getByRole('dialog', { name: 'Copy “a.pdf” to' })).toBeInTheDocument();
  });

  it('says Move and counts the items for a move', () => {
    open({ kind: 'move', from: '/work', names: ['a', 'b', 'c'], to: '/x' });

    expect(screen.getByRole('dialog', { name: 'Move 3 items to' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Move' })).toBeInTheDocument();
  });

  it('has the target filled in and focused, and verification off', () => {
    const { field } = open(copy(['a.pdf'], '/backup'));

    expect(field).toHaveValue('/backup');
    expect(field).toHaveFocus();
    expect(screen.getByRole('checkbox', { name: 'Verify after copy' })).not.toBeChecked();
  });

  it('Enter starts the job with exactly what was asked, and tells the owner', async () => {
    const { field, invoke, onDone, onClose } = open(copy(['a.pdf', 'b.txt']));
    await fireEvent.input(field, { target: { value: '/elsewhere' } });
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Verify after copy' }));

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(invoke).toHaveBeenCalledWith('ops_start', {
      kind: 'copy',
      from: '/work',
      names: ['a.pdf', 'b.txt'],
      to: '/elsewhere',
      verify: true,
    });
    expect(onClose).not.toHaveBeenCalled();
  });

  it('the primary button starts it too; Enter in the checkbox as well', async () => {
    const { invoke, onDone } = open(copy(['a.pdf']));

    await fireEvent.keyDown(screen.getByRole('checkbox'), { key: 'Enter' });
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledTimes(2));

    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it('Escape and Cancel close it without starting anything', async () => {
    const { invoke, onClose } = open(copy(['a.pdf']));

    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onClose).toHaveBeenCalledTimes(2);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('Escape and Cancel do nothing while the job is being started, and the owner still hears of it', async () => {
    let started: () => void = () => {};
    const slow = () => new Promise<void>((resolve) => (started = resolve));
    const { field, onClose, onDone } = open(copy(['a.pdf']), { ops_start: slow });

    await fireEvent.keyDown(field, { key: 'Enter' });
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onClose).not.toHaveBeenCalled();

    started();
    await vi.waitFor(() => expect(onDone).toHaveBeenCalledOnce());
    expect(onClose).not.toHaveBeenCalled();
  });

  it('an empty target is not started', async () => {
    const { field, invoke } = open(copy(['a.pdf'], ''));

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(invoke).not.toHaveBeenCalled();
  });

  it('a refusal is shown in the dialog, which stays open and can try again', async () => {
    const refuse = async () => {
      throw new IpcFailure({ code: 'busy', message: 'Too many jobs are running.' });
    };
    const { field, onDone, onClose } = open(copy(['a.pdf']), { ops_start: refuse });

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(await screen.findByRole('alert')).toHaveTextContent('Too many jobs are running.');
    expect(onDone).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Copy' })).toBeEnabled();
  });
});

describe('OpDialog rename', () => {
  const rename = (name = 'report.pdf', dir = false): OpRequest => ({ kind: 'rename', path: '/work', name, dir });

  it('shows only the name, with the stem selected and not the extension', () => {
    const { field } = open(rename());

    expect(screen.getByRole('dialog', { name: 'Rename' })).toBeInTheDocument();
    expect(field).toHaveValue('report.pdf');
    expect([field.selectionStart, field.selectionEnd]).toEqual([0, 'report'.length]);
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
  });

  it('selects the whole name of a folder', () => {
    const { field } = open(rename('v1.2', true));

    expect([field.selectionStart, field.selectionEnd]).toEqual([0, 4]);
  });

  it('Enter renames in place and reports the new name', async () => {
    const { field, invoke, onDone } = open(rename());
    await fireEvent.input(field, { target: { value: 'final.pdf' } });

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith('final.pdf'));
    expect(invoke).toHaveBeenCalledWith('ops_rename', { path: '/work', from: 'report.pdf', to: 'final.pdf' });
  });

  it('the unchanged name just closes, without asking the backend', async () => {
    const { field, invoke, onDone, onClose } = open(rename());

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(onDone).toHaveBeenCalledWith('report.pdf');
    expect(onClose).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('an existing name is refused with the backend`s message, and the dialog stays', async () => {
    const exists = async () => {
      throw new IpcFailure({ code: 'invalidInput', message: 'A file with that name already exists.' });
    };
    const { field, onDone } = open(rename(), { ops_rename: exists });
    await fireEvent.input(field, { target: { value: 'taken.pdf' } });

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(await screen.findByRole('alert')).toHaveTextContent('A file with that name already exists.');
    expect(onDone).not.toHaveBeenCalled();
  });
});
