import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import DialogFooter from './DialogFooter.svelte';
import FooterHost from '../../../tests/FooterHost.svelte';

describe('DialogFooter', () => {
  it('has a confirm and a cancel button that do what they say', async () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(DialogFooter, { busy: false, confirm: 'Create', onConfirm, onCancel });

    await fireEvent.click(screen.getByRole('button', { name: 'Create' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onConfirm).toHaveBeenCalledOnce();
    expect(onCancel).toHaveBeenCalledOnce();
    expect(screen.getByRole('button', { name: 'Create' })).toHaveClass('btn-primary');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('shows why the last try was refused', () => {
    render(DialogFooter, { busy: false, confirm: 'Create', failure: 'That name is taken.', onCancel: vi.fn() });

    expect(screen.getByRole('alert')).toHaveTextContent('That name is taken.');
  });

  it('shows a refusal that names a file with the characters that reorder text written out', () => {
    render(DialogFooter, { busy: false, confirm: 'Create', failure: 'Cannot write a\u202Etxt.exe', onCancel: vi.fn() });

    expect(screen.getByRole('alert')).toHaveTextContent('Cannot write a⟨U+202E⟩txt.exe');
  });

  it('turns both buttons off while busy', () => {
    render(DialogFooter, { busy: true, confirm: 'Create', onCancel: vi.fn() });

    expect(screen.getByRole('button', { name: 'Create' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
  });

  it('a dialog with nothing to confirm has only the close button', () => {
    render(DialogFooter, { busy: false, cancel: 'Close', onCancel: vi.fn() });

    expect(screen.getAllByRole('button').map((button) => button.textContent)).toEqual(['Close']);
  });

  it('can have a third button between the two', async () => {
    const onAlternate = vi.fn();
    render(DialogFooter, { busy: false, confirm: 'Save', alternate: "Don't save", onAlternate, onCancel: vi.fn() });

    await fireEvent.click(screen.getByRole('button', { name: "Don't save" }));

    expect(onAlternate).toHaveBeenCalledOnce();
    expect(screen.getAllByRole('button').map((button) => button.textContent)).toEqual(['Cancel', "Don't save", 'Save']);
  });

  it('marks the danger button, and the one that takes the focus', () => {
    render(FooterHost, { danger: true, focus: 'cancel' });

    expect(screen.getByRole('button', { name: 'Delete' })).toHaveClass('btn-danger');
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();
  });
});
