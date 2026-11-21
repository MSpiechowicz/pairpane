import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { IpcFailure } from '$lib/ipc/transport';

import DeleteDialog from './DeleteDialog.svelte';

function open(permanent: boolean, names = ['a.pdf'], onConfirm: () => Promise<void> = async () => {}) {
  const onClose = vi.fn();
  const confirm = vi.fn(onConfirm);
  render(DeleteDialog, { permanent, names, onConfirm: confirm, onClose });
  return { onClose, confirm };
}

afterEach(() => vi.restoreAllMocks());

describe('DeleteDialog to the Trash', () => {
  it('names the item or counts the items, and has the confirm button focused', () => {
    open(false, ['a.pdf']);

    expect(screen.getByRole('dialog', { name: 'Move “a.pdf” to the Trash?' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Move to Trash' })).toHaveFocus();
    expect(screen.queryByText('This cannot be undone.')).not.toBeInTheDocument();
  });

  it('counts several items', () => {
    open(false, ['a', 'b', 'c']);

    expect(screen.getByRole('dialog', { name: 'Move 3 items to the Trash?' })).toBeInTheDocument();
  });

  it('calls the Recycle Bin by its name on Windows', () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue('Mozilla/5.0 (Windows NT 10.0; Win64; x64)');
    open(false, ['a', 'b']);

    expect(screen.getByRole('dialog', { name: 'Move 2 items to the Recycle Bin?' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Move to Recycle Bin' })).toBeInTheDocument();
  });

  it('confirming does it once, and the button is off while it runs', async () => {
    let finish: () => void = () => {};
    const { confirm, onClose } = open(false, ['a.pdf'], () => new Promise<void>((resolve) => (finish = resolve)));

    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));

    expect(confirm).toHaveBeenCalledOnce();
    expect(screen.getByRole('button', { name: 'Move to Trash' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).not.toHaveBeenCalled();
    finish();
  });

  it('a refusal is shown, and the dialog can try again or be left', async () => {
    const { confirm, onClose } = open(false, ['a.pdf'], async () => {
      throw new IpcFailure({ code: 'busy', message: 'Too many things are running.' });
    });

    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('Too many things are running.');
    expect(screen.getByRole('button', { name: 'Move to Trash' })).toBeEnabled();
    await fireEvent.click(screen.getByRole('button', { name: 'Move to Trash' }));
    expect(confirm).toHaveBeenCalledTimes(2);

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledOnce();
  });
});

describe('DeleteDialog for good', () => {
  it('warns that it cannot be undone, with a danger button, and has Cancel focused', () => {
    open(true, ['a.pdf']);

    expect(screen.getByRole('dialog', { name: 'Delete “a.pdf” permanently?' })).toBeInTheDocument();
    expect(screen.getByText('This cannot be undone.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();
    expect(screen.getByRole('button', { name: 'Delete' })).toHaveClass('btn-danger');
  });

  it('Delete confirms, Cancel and Escape do not', async () => {
    const { confirm, onClose } = open(true, ['a', 'b']);
    expect(screen.getByRole('dialog', { name: 'Delete 2 items permanently?' })).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(2);
    expect(confirm).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(confirm).toHaveBeenCalledOnce();
  });
});
