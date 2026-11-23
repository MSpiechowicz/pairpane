import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import { Palette as PaletteModel } from './model.svelte';
import Palette from './Palette.svelte';

function open() {
  const palette = new PaletteModel({ folders: () => ['/srv/data'], names: () => ['notes.txt'] });
  palette.show();
  const onTake = vi.fn();
  render(Palette, { palette, onTake });
  return { palette, onTake, field: screen.getByRole('combobox', { name: 'Command, folder or name' }) };
}

describe('Palette', () => {
  it('has the focus in its field and shows each command with its keys', () => {
    const { field } = open();

    expect(field).toHaveFocus();
    const copy = screen.getByRole('option', { name: /^Copy F5$/ });
    expect(copy).toHaveAttribute('aria-selected', 'true');
  });

  it('filters as it is typed, moves with the arrows and takes the choice with Enter', async () => {
    const { field, onTake } = open();

    await fireEvent.input(field, { target: { value: 'notes' } });
    expect(screen.getByRole('option', { name: /notes\.txt/ })).toHaveAttribute('aria-selected', 'true');

    await fireEvent.input(field, { target: { value: 'new' } });
    await fireEvent.keyDown(field, { key: 'ArrowDown' });
    const chosen = screen.getAllByRole('option')[1];
    expect(chosen).toHaveAttribute('aria-selected', 'true');
    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(onTake).toHaveBeenCalledTimes(1);
  });

  it('takes a row that is clicked, and says when nothing matches', async () => {
    const { field, onTake } = open();

    await fireEvent.input(field, { target: { value: 'data' } });
    await fireEvent.click(screen.getByRole('option', { name: /\/srv\/data/ }));
    expect(onTake).toHaveBeenCalledWith(expect.objectContaining({ kind: 'folder', path: '/srv/data' }));
  });

  it('leaves the keys of an input method that composes text to it', async () => {
    const { field, onTake } = open();

    await fireEvent.keyDown(field, { key: 'Enter', isComposing: true });
    await fireEvent.keyDown(field, { key: 'Enter', keyCode: 229 });

    expect(onTake).not.toHaveBeenCalled();
  });

  it('says when nothing matches', async () => {
    const { field } = open();

    await fireEvent.input(field, { target: { value: 'qzxw' } });

    expect(screen.getByText('Nothing matches.')).toBeInTheDocument();
  });
});
