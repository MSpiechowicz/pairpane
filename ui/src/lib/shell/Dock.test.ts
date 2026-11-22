import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import Dock from './Dock.svelte';

describe('Dock', () => {
  it('a click on a function key reports which one', async () => {
    const onKey = vi.fn();
    render(Dock, { onKey });

    await fireEvent.click(screen.getByRole('button', { name: 'Copy (F5)' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Move (F6)' }));

    expect(onKey.mock.calls).toEqual([['F5'], ['F6']]);
  });

  it('F7 and F8 report too', async () => {
    const onKey = vi.fn();
    render(Dock, { onKey });

    await fireEvent.click(screen.getByRole('button', { name: 'New folder (F7)' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Delete (F8)' }));

    expect(onKey.mock.calls).toEqual([['F7'], ['F8']]);
  });

  it('the keys can be clicked with nobody listening', async () => {
    render(Dock);

    await fireEvent.click(screen.getByRole('button', { name: 'Delete (F8)' }));
  });

  it('F5 is called Extract inside an archive and Copy elsewhere, and still reports F5', async () => {
    const onKey = vi.fn();
    const { rerender } = render(Dock, { onKey });
    expect(screen.getByRole('button', { name: 'Copy (F5)' })).toBeInTheDocument();

    await rerender({ onKey, inArchive: true });

    expect(screen.queryByRole('button', { name: 'Copy (F5)' })).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Extract (F5)' }));
    expect(onKey.mock.calls).toEqual([['F5']]);
    expect(screen.getByRole('button', { name: 'Move (F6)' })).toBeInTheDocument();
  });

  it('over search results Move, Edit and New folder are marked as unavailable, with why; a click still reports, so the card can say it', async () => {
    const onKey = vi.fn();
    render(Dock, { onKey, inResults: true });

    const move = screen.getByRole('button', { name: 'Move (F6)' });
    const folder = screen.getByRole('button', { name: 'New folder (F7)' });
    const edit = screen.getByRole('button', { name: 'Edit (F4)' });
    for (const key of [move, folder, edit]) {
      expect(key).toHaveAttribute('aria-disabled', 'true');
    }

    expect(edit).toHaveAttribute('title', 'Open the file’s folder first.');
    expect(move).toHaveAttribute('title', 'Open the file’s folder first.');
    for (const key of ['Copy (F5)', 'Delete (F8)', 'View (F3)']) {
      expect(screen.getByRole('button', { name: key })).not.toHaveAttribute('aria-disabled');
    }

    await fireEvent.click(move);
    expect(onKey.mock.calls).toEqual([['F6']]);
  });

  it('in a folder no key is marked as unavailable', () => {
    render(Dock);

    expect(document.querySelectorAll('[aria-disabled]')).toHaveLength(0);
  });
});
