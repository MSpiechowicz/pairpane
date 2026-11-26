// Fixtures for the Devices screen: a backend whose `devices_*` commands the test answers and whose channel it drives.
import type { DeviceStatus } from '$lib/ipc/bindings/DeviceStatus';
import type { DevicesEvent } from '$lib/ipc/bindings/DevicesEvent';
import type { DevicesOverview } from '$lib/ipc/bindings/DevicesOverview';
import type { PairedDevice } from '$lib/ipc/bindings/PairedDevice';
import type { PairingShown } from '$lib/ipc/bindings/PairingShown';

import { channelBackend } from './channels';

export const ONE = 'a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6';
export const TWO = '0f9e8d7c6b5a49382716f5e4d3c2b1a0';

export const paired = (nodeId: string, name: string): PairedDevice => ({ nodeId, name, pairedAt: 1_700_000_000_000 });

export const overviewOf = (...devices: PairedDevice[]): DevicesOverview => ({
  own: { nodeId: 'ffeeddccbbaa99887766554433221100', name: 'Desk', keyStorage: 'file' },
  devices,
});

export const offered = (changes: Partial<PairingShown> = {}): PairingShown => ({
  code: '482913',
  expiresInMs: 90_000,
  target: 'node-target-text',
  qr: { size: 21, path: 'M0 0h1v1h-1z' },
  ...changes,
});

type Answers = Parameters<typeof channelBackend>[1];

/** A backend with these paired devices; `answers` overrides what a command says. `emit` sends an event on the newest channel. */
export function devicesBackend(devices: PairedDevice[] = [], answers: Answers = {}) {
  const online: DeviceStatus = { kind: 'online', name: 'Laptop' };
  const backend = channelBackend<{ onEvent: (event: DevicesEvent) => void; session: number }, DevicesEvent>('devices_open', {
    devices_open: () => overviewOf(...devices),
    devices_status: () => online,
    devices_discovered: () => [],
    devices_pair_begin: () => offered(),
    devices_pair_cancel: () => true,
    ...answers,
  });

  return { ...backend, emit: (event: DevicesEvent) => backend.started.at(-1)!.emit(event), count: (name: string) => backend.calls(name as never).length };
}
