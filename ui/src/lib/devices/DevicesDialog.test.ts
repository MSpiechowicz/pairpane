import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { IpcFailure } from '$lib/ipc/transport';

import { devicesBackend, ONE, overviewOf, paired, TWO } from '../../../tests/devices';
import DevicesDialog from './DevicesDialog.svelte';
import { TRAY_NOTE } from './format';

const laptop = paired(ONE, 'Laptop');
const phone = paired(TWO, 'Phone');

async function open(devices = [laptop], answers: Parameters<typeof devicesBackend>[1] = {}) {
  const backend = devicesBackend(devices, answers);
  const onClose = vi.fn();
  const view = render(DevicesDialog, { ipc: backend.ipc, onClose });
  await screen.findByRole('group', { name: 'This device' });

  return { backend, onClose, view };
}

const dialog = () => screen.getByRole('dialog', { name: 'Devices' });
const row = (name: string) => screen.getByRole('button', { name: `Rename ${name}` }).closest('li') as HTMLElement;

beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
afterEach(() => vi.useRealTimers());

describe('this device', () => {
  it('shows its name, its short id, where the key is kept and what closing the window does', async () => {
    await open();

    const own = screen.getByRole('group', { name: 'This device' });
    expect(own).toHaveTextContent('Desk');
    expect(own).toHaveTextContent('ffeedd…1100');
    expect(own).toHaveTextContent("This system has no keychain (Secret Service), so the device key is kept in a private file (readable only by you) in Pairpane's data folder.");
    expect(own).toHaveTextContent(TRAY_NOTE);
  });

  it('opens with the Close button focused, and Close closes', async () => {
    const { onClose } = await open();
    expect(screen.getByRole('button', { name: 'Close' })).toHaveFocus();

    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe('when it cannot be opened', () => {
  it('says why, and Try again opens it again', async () => {
    let fail = true;
    const backend = devicesBackend([], {
      devices_open: () => (fail ? Promise.reject(new IpcFailure({ code: 'internal', message: 'The device link is not running.' })) : overviewOf()),
    });
    render(DevicesDialog, { ipc: backend.ipc, onClose: vi.fn() });
    expect(await screen.findByRole('alert')).toHaveTextContent('The device link is not running.');

    fail = false;
    await fireEvent.click(screen.getByRole('button', { name: 'Try again' }));

    expect(await screen.findByRole('group', { name: 'This device' })).toBeInTheDocument();
    expect(backend.count('devices_open')).toBe(2);
  });
});

describe('the paired devices', () => {
  it('lists them with their short id and status', async () => {
    await open([laptop, phone], { devices_status: (args: { nodeId: string }) => (args.nodeId === ONE ? { kind: 'online', name: 'Laptop' } : { kind: 'offline' }) });

    await waitFor(() => expect(row('Laptop')).toHaveTextContent('Online'));
    expect(row('Laptop')).toHaveTextContent('a1b2c3…c5d6');
    await waitFor(() => expect(row('Phone')).toHaveTextContent('Offline'));
  });

  it('says so when there are none', async () => {
    await open([]);

    expect(screen.getByText('No device is paired yet.')).toBeInTheDocument();
  });

  it('Rename: Enter saves what was typed', async () => {
    const { backend } = await open([laptop], { devices_rename: () => [paired(ONE, 'Work')] });
    await fireEvent.click(screen.getByRole('button', { name: 'Rename Laptop' }));

    const field = screen.getByRole('textbox', { name: 'New name of Laptop' });
    expect(field).toHaveFocus();
    await fireEvent.input(field, { target: { value: '  Work ' } });
    await fireEvent.keyDown(field, { key: 'Enter' });

    await waitFor(() => expect(row('Work')).toBeInTheDocument());
    expect(backend.calls('devices_rename')).toEqual([{ nodeId: ONE, name: 'Work' }]);
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
  });

  it('Rename: Escape ends the renaming and keeps the dialog', async () => {
    const { backend, onClose } = await open();
    await fireEvent.click(screen.getByRole('button', { name: 'Rename Laptop' }));

    await fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Escape' });

    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    expect(dialog()).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
    expect(backend.count('devices_rename')).toBe(0);
  });

  it('Rename: a refusal is told and the field stays', async () => {
    await open([laptop], { devices_rename: () => Promise.reject(new IpcFailure({ code: 'invalidInput', message: 'That name is not valid.' })) });
    await fireEvent.click(screen.getByRole('button', { name: 'Rename Laptop' }));
    await fireEvent.input(screen.getByRole('textbox'), { target: { value: 'x' } });

    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    expect(await within(dialog()).findByRole('alert')).toHaveTextContent('That name is not valid.');
    expect(screen.getByRole('textbox')).toBeInTheDocument();
  });

  it('Unpair asks first; Cancel keeps the device and Unpair removes it', async () => {
    const { backend } = await open([laptop, phone], { devices_unpair: () => [phone] });

    await fireEvent.click(screen.getByRole('button', { name: 'Unpair Laptop' }));
    const question = screen.getByRole('dialog', { name: 'Unpair Laptop?' });
    await fireEvent.click(within(question).getByRole('button', { name: 'Cancel' }));
    expect(backend.count('devices_unpair')).toBe(0);

    await fireEvent.click(screen.getByRole('button', { name: 'Unpair Laptop' }));
    await fireEvent.click(within(screen.getByRole('dialog', { name: 'Unpair Laptop?' })).getByRole('button', { name: 'Unpair' }));

    await waitFor(() => expect(screen.queryByRole('button', { name: 'Rename Laptop' })).not.toBeInTheDocument());
    expect(backend.calls('devices_unpair')).toEqual([{ nodeId: ONE }]);
    expect(row('Phone')).toBeInTheDocument();
  });

  it('follows what the backend reports on its channel', async () => {
    const { backend } = await open([laptop]);

    backend.emit({ kind: 'paired', device: phone });

    expect(await screen.findByRole('button', { name: 'Rename Phone' })).toBeInTheDocument();
  });
});

describe('pairing', () => {
  it('Pair a device shows the code, and Cancel withdraws it', async () => {
    const { backend } = await open([]);

    await fireEvent.click(screen.getByRole('button', { name: 'Pair a device' }));

    expect(await screen.findByLabelText('Pairing code')).toHaveTextContent('482913');
    await fireEvent.click(within(screen.getByRole('region', { name: 'Pair a device' })).getByRole('button', { name: 'Cancel' }));
    expect(backend.count('devices_pair_cancel')).toBe(1);
    expect(screen.queryByLabelText('Pairing code')).not.toBeInTheDocument();
  });

  it('a device that pairs with the code is told in a notice', async () => {
    const { backend } = await open([]);
    await fireEvent.click(screen.getByRole('button', { name: 'Pair a device' }));
    await screen.findByLabelText('Pairing code');

    backend.emit({ kind: 'paired', device: phone });

    expect(await screen.findByRole('status')).toHaveTextContent('Paired with Phone.');
    expect(screen.queryByLabelText('Pairing code')).not.toBeInTheDocument();
  });

  it('closing the dialog withdraws the code', async () => {
    const { backend, view } = await open([]);
    await fireEvent.click(screen.getByRole('button', { name: 'Pair a device' }));
    await screen.findByLabelText('Pairing code');

    view.unmount();

    expect(backend.count('devices_pair_cancel')).toBe(1);
  });
});

describe('joining', () => {
  it('shows the devices found, joins with the typed code and lists the new device', async () => {
    const { backend } = await open([], {
      devices_discovered: () => [{ nodeId: TWO, name: 'Phone', paired: false }],
      devices_join: () => phone,
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Join with a code' }));

    await fireEvent.click(await screen.findByRole('button', { name: 'Use Phone' }));
    await fireEvent.input(screen.getByLabelText('Code'), { target: { value: '654321' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Join' }));

    expect(await screen.findByRole('button', { name: 'Rename Phone' })).toBeInTheDocument();
    expect(backend.calls('devices_join')).toEqual([{ code: '654321', target: TWO }]);
    expect(screen.getByRole('status')).toHaveTextContent('Paired with Phone.');
  });

  it('tells why a join was refused', async () => {
    await open([], { devices_join: () => Promise.reject(new IpcFailure({ code: 'invalidInput', message: 'The code is not right.' })) });
    await fireEvent.click(screen.getByRole('button', { name: 'Join with a code' }));
    await fireEvent.input(screen.getByLabelText('Address or id'), { target: { value: 'node-x' } });
    await fireEvent.input(screen.getByLabelText('Code'), { target: { value: '1' } });

    await fireEvent.click(screen.getByRole('button', { name: 'Join' }));

    expect(await within(dialog()).findByRole('alert')).toHaveTextContent('The code is not right.');
  });

  it('stops searching when the dialog goes', async () => {
    const { backend, view } = await open([]);
    await fireEvent.click(screen.getByRole('button', { name: 'Join with a code' }));
    await waitFor(() => expect(backend.count('devices_discovered')).toBe(1));

    view.unmount();
    await vi.advanceTimersByTimeAsync(10_000);

    expect(backend.count('devices_discovered')).toBe(1);
    expect(vi.getTimerCount()).toBe(0);
  });
});
