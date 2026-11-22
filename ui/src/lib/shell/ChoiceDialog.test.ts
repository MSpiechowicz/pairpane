import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import ChoiceDialog from './ChoiceDialog.svelte';

function open(extra: Record<string, unknown> = {}) {
  const handlers = { onConfirm: vi.fn(), onAlternate: vi.fn(), onCancel: vi.fn() };
  render(ChoiceDialog, { title: 'Save changes?', message: 'They are lost otherwise.', confirm: 'Save', alternate: "Don't save", ...handlers, ...extra });

  return handlers;
}

describe('ChoiceDialog', () => {
  it('is a dialog with its question and each choice a button that does what it says', async () => {
    const { onConfirm, onAlternate, onCancel } = open();
    expect(screen.getByRole('dialog', { name: 'Save changes?' })).toBeInTheDocument();
    expect(screen.getByText('They are lost otherwise.')).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await fireEvent.click(screen.getByRole('button', { name: "Don't save" }));
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect([onConfirm, onAlternate, onCancel].map((handler) => handler.mock.calls.length)).toEqual([1, 1, 1]);
  });

  it('shows the title and the message with the characters that reorder text written out', () => {
    open({ title: 'Save “a\u202Etxt.exe”?', message: 'Not saved: b\u200Bc' });

    expect(screen.getByRole('dialog', { name: 'Save “a⟨U+202E⟩txt.exe”?' })).toBeInTheDocument();
    expect(screen.getByText('Not saved: b⟨U+200B⟩c')).toBeInTheDocument();
  });

  it('Escape is Cancel', async () => {
    const { onCancel, onConfirm } = open();

    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(onCancel).toHaveBeenCalledOnce();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it('has two choices when there is no alternative', () => {
    open({ alternate: undefined, confirm: 'Save anyway' });

    expect(screen.getAllByRole('button').map((button) => button.textContent)).toEqual(['Cancel', 'Save anyway']);
  });

  it('the focus starts on the first choice', () => {
    open();
    expect(screen.getByRole('button', { name: 'Save' })).toHaveFocus();
  });

  it('the focus starts on Cancel for a choice that cannot be taken back', () => {
    open({ confirm: 'Overwrite', danger: true });

    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();
    expect(screen.getByRole('button', { name: 'Overwrite' })).toHaveClass('btn-danger');
  });
});
