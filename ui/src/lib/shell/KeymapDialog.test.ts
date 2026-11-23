import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import { Keymap } from './keymap.svelte';
import KeymapDialog from './KeymapDialog.svelte';

function open() {
  const keymap = new Keymap();
  const onClose = vi.fn();
  render(KeymapDialog, { keymap, onClose });
  const row = (label: string) => screen.getByRole('row', { name: new RegExp(`^${label}`) });
  return { keymap, onClose, row };
}

describe('KeymapDialog', () => {
  it('lists every action with its keys', () => {
    const { row } = open();

    expect(row('Copy')).toHaveTextContent('F5');
    expect(row('Command palette')).toHaveTextContent('Ctrl+K');
  });

  it('takes the next keys pressed as the new shortcut, and says from which action it took them', async () => {
    const { keymap, row } = open();

    await fireEvent.click(screen.getByRole('button', { name: 'Change the keys of Copy' }));
    const cancel = screen.getByRole('button', { name: 'Cancel' });
    expect(cancel).toHaveFocus();
    await fireEvent.keyDown(cancel, { key: 'Control', ctrlKey: true });
    await fireEvent.keyDown(cancel, { key: 'F6' });

    expect(keymap.keysOf('copy')).toBe('F6');
    expect(screen.getByRole('status')).toHaveTextContent('Copy: F6, taken from “Move”.');
    await vi.waitFor(() => expect(screen.getByRole('button', { name: 'Change the keys of Copy' })).toHaveFocus());
    expect(screen.getByRole('button', { name: 'Reset the keys of Move' })).toBeEnabled();
    expect(row('Move')).toHaveTextContent('No key');
    expect(screen.getByRole('button', { name: 'Reset the keys of Copy' })).toBeEnabled();
  });

  it('says why a key is refused and keeps waiting; Escape gives up', async () => {
    const { keymap } = open();

    await fireEvent.click(screen.getByRole('button', { name: 'Change the keys of Copy' }));
    await fireEvent.keyDown(screen.getByRole('button', { name: 'Cancel' }), { key: 'c' });
    expect(screen.getByRole('status')).toHaveTextContent('Hold Ctrl or Alt');

    await fireEvent.keyDown(screen.getByRole('button', { name: 'Cancel' }), { key: 'Escape' });
    expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
    expect(keymap.keysOf('copy')).toBe('F5');
  });

  it('stops waiting when the focus leaves, and says Cmd and AltGr are not keys; a held key is not the new one', async () => {
    const { keymap } = open();

    await fireEvent.click(screen.getByRole('button', { name: 'Change the keys of Copy' }));
    const cancel = screen.getByRole('button', { name: 'Cancel' });
    await fireEvent.keyDown(cancel, { key: 'Enter', repeat: true });
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
    await fireEvent.keyDown(cancel, { key: 'k', metaKey: true });
    expect(screen.getByRole('status')).toHaveTextContent('Cmd, AltGr and Ctrl+Alt');

    await fireEvent.blur(cancel);

    expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
    expect(keymap.keysOf('copy')).toBe('F5');
  });

  it('resets one action or all of them', async () => {
    const { keymap } = open();
    keymap.set('copy', { key: 'y', ctrl: true });
    keymap.set('move', { key: 'u', ctrl: true });

    await fireEvent.click(await screen.findByRole('button', { name: 'Reset the keys of Copy' }));
    expect(keymap.keysOf('copy')).toBe('F5');

    await fireEvent.click(screen.getByRole('button', { name: 'Reset all to the defaults' }));
    expect(keymap.keysOf('move')).toBe('F6');
  });
});
