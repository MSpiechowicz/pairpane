import { fireEvent, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ShellEvent } from '$lib/ipc/bindings/ShellEvent';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { appShown, key, listbox, withArchiveOnRight } from '../tests/app';

const input = () => screen.getByRole('textbox', { name: 'Shell command' });

async function runLine(text: string) {
  await fireEvent.input(input(), { target: { value: text } });
  await fireEvent.keyDown(input(), { key: 'Enter' });
  await tick();
}

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('the command line', () => {
  it('runs a line in the active pane`s folder and shows what it prints', async () => {
    const backend = await appShown(invoke, { shell_run: () => 5 });

    await runLine('ls');

    await vi.waitFor(() => expect(backend.sent('shell_run')).toHaveLength(1));
    const [args] = backend.sent('shell_run') as { path: string; command: string; onEvent: (event: ShellEvent) => void }[];
    expect(args).toMatchObject({ path: '/left', command: 'ls' });
    args.onEvent({ kind: 'output', text: 'a.pdf\n' });
    args.onEvent({ kind: 'done', code: 0, stopped: false, truncated: false });

    const output = await screen.findByRole('region', { name: 'Command output' });
    expect(output).toHaveTextContent('a.pdf');
    expect(output).toHaveTextContent('Exit code 0');
  });

  it('does not run inside an archive and says so', async () => {
    const backend = await withArchiveOnRight(invoke, 'a.zip');
    // The helper leaves the left pane active: the archive's pane is made the active one.
    await key(listbox('Left'), 'Tab');

    await runLine('ls');

    expect(backend.sent('shell_run')).toHaveLength(0);
    expect(await screen.findByText('Commands run in a folder on disk: open one first.')).toBeInTheDocument();
  });

  it('the palette opens a terminal in the active folder', async () => {
    const backend = await appShown(invoke, { terminal_open: () => null });

    await key(listbox('Left'), 'k', { ctrlKey: true });
    const field = screen.getByRole('combobox', { name: 'Command, folder or name' });
    await fireEvent.input(field, { target: { value: 'open terminal' } });
    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(backend.sent('terminal_open')).toEqual([{ path: '/left' }]));
  });
});
