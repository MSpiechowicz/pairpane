import { fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { appShown, key, listbox, pane } from '../tests/app';

const field = () => screen.getByRole('combobox', { name: 'Command, folder or name' });

async function typed(text: string) {
  await fireEvent.input(field(), { target: { value: text } });
  await fireEvent.keyDown(field(), { key: 'Enter' });
  await tick();
}

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('the command palette', () => {
  it('Ctrl+K opens it; Esc closes it and the list has the focus again', async () => {
    await appShown(invoke);

    await key(listbox('Left'), 'k', { ctrlKey: true });
    expect(screen.getByRole('dialog', { name: 'Command palette' })).toBeInTheDocument();
    expect(field()).toHaveFocus();

    await key(field(), 'Escape');

    expect(screen.queryByRole('dialog', { name: 'Command palette' })).not.toBeInTheDocument();
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('runs a command the way its shortcut does', async () => {
    await appShown(invoke);
    await key(listbox('Left'), 'k', { ctrlKey: true });

    await typed('new folder');

    expect(screen.getByRole('dialog', { name: /folder/i })).toBeInTheDocument();
  });

  it('puts the cursor on a name of the active folder', async () => {
    await appShown(invoke);
    await key(listbox('Left'), 'k', { ctrlKey: true });

    await typed('b.txt');

    await vi.waitFor(() => expect(within(pane('Left pane')).getByRole('option', { name: /^b/ })).toHaveClass('cursor'));
  });
});

describe('the keyboard shortcuts', () => {
  it('are changed from the palette, take effect at once and are saved', async () => {
    const backend = await appShown(invoke);
    await key(listbox('Left'), 'k', { ctrlKey: true });
    await typed('keyboard shortcuts');

    await fireEvent.click(screen.getByRole('button', { name: 'Change the keys of Copy' }));
    await fireEvent.keyDown(screen.getByRole('button', { name: 'Cancel' }), { key: 'y', ctrlKey: true });
    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());

    expect(await key(listbox('Left'), 'y', { ctrlKey: true })).toBe(false);
    expect(screen.getByRole('dialog', { name: /^Copy/ })).toBeInTheDocument();
    await vi.waitFor(() =>
      expect(backend.sent('state_save').at(-1)).toMatchObject({
        state: { keys: [{ action: 'copy', keys: [{ key: 'y', ctrl: true }] }] },
      }),
    );
  });
});
