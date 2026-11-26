import { fireEvent, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { IpcFailure } from '$lib/ipc/transport';

import { appShown, key, listbox } from '../tests/app';
import { offered, overviewOf, ONE, paired } from '../tests/devices';

const answers = {
  devices_open: () => overviewOf(paired(ONE, 'Laptop')),
  devices_status: () => ({ kind: 'online', name: 'Laptop' }),
  devices_pair_begin: () => offered(),
};

const dialog = () => screen.findByRole('dialog', { name: 'Devices' });

async function openFromMenu() {
  await fireEvent.click(screen.getByRole('button', { name: /^Settings/ }));
  await fireEvent.click(screen.getByRole('menuitem', { name: 'Devices…' }));
}

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
});

describe('the Devices screen', () => {
  it('asks nothing of the device link until it is opened', async () => {
    const backend = await appShown(invoke, answers);

    expect(invoke.mock.calls.filter(([name]) => String(name).startsWith('devices_'))).toEqual([]);
    expect(backend.sent('devices_open')).toEqual([]);
  });

  it('is an item of the Settings menu that opens the screen', async () => {
    const backend = await appShown(invoke, answers);

    await openFromMenu();

    expect(await dialog()).toBeInTheDocument();
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    expect(await screen.findByRole('button', { name: 'Rename Laptop' })).toBeInTheDocument();
    expect(backend.sent('devices_open')).toHaveLength(1);
  });

  it('is a command of the palette', async () => {
    const backend = await appShown(invoke, answers);
    await key(listbox('Left'), 'k', { ctrlKey: true });

    const field = screen.getByRole('combobox', { name: 'Command, folder or name' });
    await fireEvent.input(field, { target: { value: 'devices…' } });
    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(await dialog()).toBeInTheDocument();
    await vi.waitFor(() => expect(backend.sent('devices_open')).toHaveLength(1));
  });

  it('leaves the keyboard to the dialog', async () => {
    const backend = await appShown(invoke, answers);
    await openFromMenu();
    await dialog();

    await key(listbox('Left'), 'F7');

    expect(screen.queryByRole('dialog', { name: 'New folder' })).not.toBeInTheDocument();
    expect(backend.sent('ops_mkdir')).toEqual([]);
  });

  it('Escape closes it and withdraws the code that is shown', async () => {
    const backend = await appShown(invoke, answers);
    await openFromMenu();
    await fireEvent.click(await screen.findByRole('button', { name: 'Pair a device' }));
    await screen.findByLabelText('Pairing code');

    await key(await dialog(), 'Escape');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('devices_pair_cancel')).toHaveLength(1);
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('closing it without a code sends no cancel', async () => {
    const backend = await appShown(invoke, answers);
    await openFromMenu();
    await dialog();

    await key(await dialog(), 'Escape');

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(backend.sent('devices_pair_cancel')).toEqual([]);
  });

  it('Quit Pairpane in the Settings menu asks the backend to quit', async () => {
    const backend = await appShown(invoke, answers);

    await fireEvent.click(screen.getByRole('button', { name: /^Settings/ }));
    const items = screen.getAllByRole('menuitem');
    expect(items.at(-1)).toHaveAccessibleName('Quit Pairpane');
    await fireEvent.click(items.at(-1)!);

    expect(backend.sent('app_quit')).toHaveLength(1);
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('Quit Pairpane is a command of the palette', async () => {
    const backend = await appShown(invoke, answers);
    await key(listbox('Left'), 'k', { ctrlKey: true });

    const field = screen.getByRole('combobox', { name: 'Command, folder or name' });
    await fireEvent.input(field, { target: { value: 'quit pair' } });
    await fireEvent.keyDown(field, { key: 'Enter' });

    await vi.waitFor(() => expect(backend.sent('app_quit')).toHaveLength(1));
  });

  it('says so when the backend refuses to quit', async () => {
    await appShown(invoke, { ...answers, app_quit: () => Promise.reject(new IpcFailure({ code: 'internal', message: 'Could not quit.' })) });

    await fireEvent.click(screen.getByRole('button', { name: /^Settings/ }));
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Quit Pairpane' }));

    expect(await screen.findByText('Could not quit.')).toBeInTheDocument();
  });
});
