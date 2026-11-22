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
});
