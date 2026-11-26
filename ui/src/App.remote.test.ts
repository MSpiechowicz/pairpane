import { fireEvent, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { REMOTE_NOTICE } from '$lib/ops/targets';
import { keymap } from '$lib/shell/keymap.svelte';

import { appShown, key, listbox, pane } from '../tests/app';
import { entry } from '../tests/panes';

const field = () => screen.getByRole('combobox', { name: 'Command, folder or name' });

/** Runs a command of the palette by its name. */
async function command(text: string) {
  await key(listbox('Left'), 'k', { ctrlKey: true });
  await fireEvent.input(field(), { target: { value: text } });
  await fireEvent.keyDown(field(), { key: 'Enter' });
  await tick();
}

const answers = {
  remote_hosts: () => [{ alias: 'work', hostName: null, user: null, port: null }],
  remote_connect: ({ secret }: { secret: string | null }) =>
    secret === null
      ? { kind: 'secretNeeded', reason: null, secret: 'password', wrong: false }
      : { kind: 'connected', site: 3, label: 'ann@work', home: '/home/ann' },
};

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
  keymap.resetAll();
});

/** Answers the Connect dialog (open) for `work` and its password question. */
async function connectToWork() {
  await fireEvent.input(await screen.findByLabelText('Host'), { target: { value: 'work' } });
  await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
  const password = await screen.findByLabelText('Password');
  await fireEvent.input(password, { target: { value: 'pw' } });
  await fireEvent.keyDown(password, { key: 'Enter' });
}

const openDrives = () => fireEvent.click(within(pane('Left pane')).getByRole('button', { name: 'Drives' }));

describe('connecting to a server', () => {
  it('the palette’s entry asks for the server, then the password, and lists the home folder in the active pane', async () => {
    const backend = await appShown(invoke, answers as never);

    await command('connect to sftp');
    await connectToWork();

    await vi.waitFor(() => expect(backend.sent('remote_open')).toHaveLength(1));
    expect(backend.sent('remote_open')[0]).toMatchObject({ pane: 'left', site: 3, dir: '/home/ann', child: null });
    expect(backend.sent('remote_connect')).toEqual([
      { target: { host: 'work', port: null, user: null, keyFile: null }, secret: null, kind: null },
      { target: { host: 'work', port: null, user: null, keyFile: null }, secret: 'pw', kind: 'password' },
    ]);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('the Drives menu has the entry too', async () => {
    await appShown(invoke, answers as never);

    await openDrives();
    await fireEvent.click(await screen.findByRole('button', { name: 'Connect to SFTP server…' }));

    expect(await screen.findByRole('dialog', { name: 'Connect to SFTP server' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Disconnect/ })).not.toBeInTheDocument();
  });
});

describe('a pane on a server', () => {
  async function connected() {
    const backend = await appShown(invoke, answers as never);
    await command('connect to sftp');
    await connectToWork();
    await vi.waitFor(() => expect(backend.sent('remote_open')).toHaveLength(1));
    backend.listRemote('left', 3, 'ann@work', '/home/ann', [entry('notes.txt')]);
    await tick();
    return backend;
  }

  it('refuses the file keys with a card and no dialog', async () => {
    await connected();

    await key(listbox('Left'), 'F7');

    expect(await screen.findByText(REMOTE_NOTICE)).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('refuses F3 with the same card and opens no viewer', async () => {
    const backend = await connected();

    await key(listbox('Left'), 'F3');

    expect(await screen.findByText(REMOTE_NOTICE)).toBeInTheDocument();
    expect(backend.sent('view_open')).toEqual([]);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('is left by Disconnect, in the Drives menu, back to the folder it was in', async () => {
    const backend = await connected();

    await openDrives();
    await fireEvent.click(await screen.findByRole('button', { name: 'Disconnect from ann@work' }));

    await vi.waitFor(() => expect(backend.sent('remote_disconnect')).toEqual([{ site: 3 }]));
    await vi.waitFor(() => expect(backend.sent('pane_open').at(-1)).toMatchObject({ pane: 'left', path: '/left' }));
  });

  it('is left by the palette’s Disconnect too', async () => {
    const backend = await connected();

    await command('disconnect');

    await vi.waitFor(() => expect(backend.sent('remote_disconnect')).toEqual([{ site: 3 }]));
  });
});
