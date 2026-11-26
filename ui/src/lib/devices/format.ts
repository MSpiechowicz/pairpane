import type { DeviceStatus } from '$lib/ipc/bindings/DeviceStatus';
import type { KeyStorage } from '$lib/ipc/bindings/KeyStorage';
import type { PairingEnd } from '$lib/ipc/bindings/PairingEnd';

/** A device's status as the screen holds it: the answer, or none yet. */
export type Status = DeviceStatus | 'checking';

/** What the Devices screen says about closing the window. */
export const TRAY_NOTE = 'Closing the window keeps Pairpane running in the tray while a device is paired. Open Pairpane again to show it, or choose Quit Pairpane in the Settings menu to stop it.';

/** A node id is long; the screen shows its start and end (the full id is a tooltip). */
export function shortId(nodeId: string): string {
  return nodeId.length <= 12 ? nodeId : `${nodeId.slice(0, 6)}…${nodeId.slice(-4)}`;
}

/** The time that is left, as `m:ss` (a part of a second counts as a whole one). */
export function countdown(millis: number): string {
  const seconds = Math.ceil(Math.max(0, millis) / 1000);

  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}

/** Where the device key is kept, in words. */
export function keyStorageNote(storage: KeyStorage): string {
  switch (storage) {
    case 'keychain':
      return 'The device key is kept in the system keychain.';
    case 'file':
      return "This system has no keychain (Secret Service), so the device key is kept in a private file (readable only by you) in Pairpane's data folder.";
    case 'memory':
      return 'The device key could not be stored, so it is kept in memory only: this device gets a new identity each time Pairpane starts.';
  }
}

/** What a row says of the device's status; `name` is the name the row shows. */
export function statusText(status: Status, name: string): string {
  if (status === 'checking') {
    return 'Checking…';
  }

  switch (status.kind) {
    case 'online':
      return status.name === name ? 'Online' : `Online as ${status.name}`;
    case 'offline':
      return 'Offline';
    case 'refused':
      return 'Refused: the other device does not accept this one (it may have unpaired it).';
  }
}

/** Why the code is gone, for the reasons that are not the user's own doing. */
export function pairingEndText(reason: PairingEnd): string {
  switch (reason) {
    case 'used':
      return 'The code was used.';
    case 'expired':
      return 'The code expired. Start again for a new one.';
    case 'cancelled':
      return 'The code was withdrawn.';
    case 'tooManyAttempts':
      return 'Too many wrong codes were tried, so the code was withdrawn. Start again for a new one.';
  }
}
