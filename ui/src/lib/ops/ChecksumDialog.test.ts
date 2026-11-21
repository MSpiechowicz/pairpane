import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import { IpcFailure } from '$lib/ipc/transport';

import { opsBackend } from '../../../tests/ops';
import ChecksumDialog from './ChecksumDialog.svelte';

function open(names = ['a.iso'], path = '/data/images', answers: Parameters<typeof opsBackend>[0] = {}) {
  const backend = opsBackend(answers);
  const onClose = vi.fn();
  const onDone = vi.fn();
  render(ChecksumDialog, { ipc: backend.ipc, path, names, onClose, onDone });
  return { ...backend, onClose, onDone, field: screen.getByRole('textbox', { name: 'Checksum file' }) as HTMLInputElement };
}

describe('ChecksumDialog', () => {
  it('names the item, offers SHA-256 first, and names the file after the item', () => {
    const { field } = open();

    expect(screen.getByRole('dialog', { name: 'Create checksum file for “a.iso”' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'SHA-256' })).toBeChecked();
    expect(screen.getByRole('radio', { name: 'BLAKE3' })).not.toBeChecked();
    expect(screen.getByRole('radio', { name: 'MD5' })).not.toBeChecked();
    expect(field).toHaveValue('a.iso.sha256');
    expect(field).toHaveFocus();
    expect([field.selectionStart, field.selectionEnd]).toEqual([0, 'a.iso'.length]);
  });

  it('counts several items and names the file after the folder', () => {
    const { field } = open(['a', 'b', 'c'], '/home/ann/photos');

    expect(screen.getByRole('dialog', { name: 'Create checksum file for 3 items' })).toBeInTheDocument();
    expect(field).toHaveValue('photos.sha256');
  });

  it('the extension follows the algorithm until the name is typed', async () => {
    const { field } = open();

    await fireEvent.click(screen.getByRole('radio', { name: 'BLAKE3' }));
    expect(field).toHaveValue('a.iso.blake3');
    await fireEvent.click(screen.getByRole('radio', { name: 'MD5' }));
    expect(field).toHaveValue('a.iso.md5');

    await fireEvent.input(field, { target: { value: 'sums.txt' } });
    await fireEvent.click(screen.getByRole('radio', { name: 'SHA-256' }));
    expect(field).toHaveValue('sums.txt');
  });

  it('Enter asks for the checksum file with exactly what was chosen, and tells the owner its name', async () => {
    const { field, calls, onDone } = open(['a.iso', 'b.iso']);
    await fireEvent.click(screen.getByRole('radio', { name: 'BLAKE3' }));

    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(onDone).toHaveBeenCalledWith('images.blake3'));
    expect(calls('ops_task')).toEqual([
      { from: '/data/images', names: ['a.iso', 'b.iso'], task: { kind: 'checksum', algorithm: 'blake3', output: 'images.blake3' } },
    ]);
  });

  it('the primary button asks too', async () => {
    const { calls } = open();

    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));

    await vi.waitFor(() => expect(calls('ops_task')).toHaveLength(1));
  });

  it('an empty name asks for nothing', async () => {
    const { field, invoke } = open();
    await fireEvent.input(field, { target: { value: '  ' } });

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(invoke).not.toHaveBeenCalled();
  });

  it('a refusal is shown, and the dialog stays', async () => {
    const { onDone, field } = open(['a.iso'], '/data', {
      ops_task: async () => {
        throw new IpcFailure({ code: 'invalidInput', message: 'the name of the checksum file must be a single plain name' });
      },
    });
    await fireEvent.input(field, { target: { value: 'x/y.sha256' } });

    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(await screen.findByRole('alert')).toHaveTextContent('must be a single plain name');
    expect(onDone).not.toHaveBeenCalled();
  });

  it('Escape and Cancel leave without asking', async () => {
    const { invoke, onClose } = open();

    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onClose).toHaveBeenCalledTimes(2);
    expect(invoke).not.toHaveBeenCalled();
  });
});
