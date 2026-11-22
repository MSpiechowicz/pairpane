import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Toast from './Toast.svelte';

function show(props: { undoable?: boolean; error?: boolean } = {}) {
  const onUndo = vi.fn();
  const onClose = vi.fn();
  render(Toast, { text: 'Moved “a.pdf” to the Trash.', onUndo, onClose, ...props });
  return { onUndo, onClose };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('Toast', () => {
  it('is a polite live region with the text and a Close button', () => {
    const { onClose } = show();

    expect(screen.getByRole('region', { name: 'Notice' })).toHaveAttribute('aria-live', 'polite');
    expect(screen.getByText('Moved “a.pdf” to the Trash.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument();

    screen.getByRole('button', { name: 'Close' }).click();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('writes out the controls and invisible characters of a name in the text', () => {
    render(Toast, { text: 'Moved “a\u202Efdp.exe” and “b\u200B.txt”.', onUndo: vi.fn(), onClose: vi.fn() });

    expect(screen.getByText('Moved “a⟨U+202E⟩fdp.exe” and “b⟨U+200B⟩.txt”.')).toBeInTheDocument();
  });

  it('Undo is there only when it can be, does not take the focus, and asks for the undo', async () => {
    const { onUndo } = show({ undoable: true });
    const undo = screen.getByRole('button', { name: 'Undo' });

    expect(undo).not.toHaveFocus();
    expect(document.body).toHaveFocus();
    await fireEvent.click(undo);

    expect(onUndo).toHaveBeenCalledOnce();
  });

  it('closes by itself after 8 seconds, not before', () => {
    const { onClose } = show();

    vi.advanceTimersByTime(7999);
    expect(onClose).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);

    expect(onClose).toHaveBeenCalledOnce();
  });

  it('waits while the pointer is over it, and for the time that was left once it leaves', async () => {
    const { onClose } = show();
    const card = screen.getByRole('region', { name: 'Notice' });

    vi.advanceTimersByTime(5000);
    await fireEvent.mouseEnter(card);
    vi.advanceTimersByTime(60_000);
    expect(onClose).not.toHaveBeenCalled();

    await fireEvent.mouseLeave(card);
    vi.advanceTimersByTime(2999);
    expect(onClose).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('waits while something in it has the focus', async () => {
    const { onClose } = show({ undoable: true });
    const undo = screen.getByRole('button', { name: 'Undo' });

    undo.focus();
    await tick();
    vi.advanceTimersByTime(60_000);
    expect(onClose).not.toHaveBeenCalled();

    screen.getByRole('button', { name: 'Close' }).focus(); // still inside the card
    await tick();
    vi.advanceTimersByTime(60_000);
    expect(onClose).not.toHaveBeenCalled();

    (document.activeElement as HTMLElement).blur();
    await tick();
    vi.advanceTimersByTime(8000);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it('shows a failure in the danger colour', () => {
    show({ error: true });

    expect(screen.getByText('Moved “a.pdf” to the Trash.')).toHaveClass('text-danger');
  });
});
