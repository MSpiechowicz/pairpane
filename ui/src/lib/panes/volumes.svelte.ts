import type { Volume } from '$lib/ipc/bindings/Volume';
import type { VolumeEvent } from '$lib/ipc/bindings/VolumeEvent';
import type { Transport } from '$lib/ipc/transport';

/** How often the list is read again while the drive menu is open (a mounted or ejected volume shows up by then). */
export const VOLUMES_POLL_MS = 2000;

/**
 * The places the drive menu offers and the path bar names its first step after. The list is read once at startup
 * and again while the drive menu is open; nothing runs in the background otherwise. A read the backend refuses
 * (it is still busy with the previous one) or that fails keeps the list as it was.
 */
export class Volumes {
  readonly #ipc: Transport;
  /** Replaced on every change, never changed in place. */
  #list = $state.raw<Volume[]>([]);
  /** Bumped by every read: the list of an older read is stale (free space of any read is still true). */
  #epoch = 0;
  #reading = false;

  constructor(ipc: Transport) {
    this.#ipc = ipc;
  }

  get list(): readonly Volume[] {
    return this.#list;
  }

  /** Reads the list. A read that is still running makes this one a no-op. */
  async refresh(): Promise<void> {
    if (this.#reading) {
      return;
    }

    const epoch = ++this.#epoch;
    this.#reading = true;
    try {
      await this.#ipc.invoke('volumes_list', { onEvent: (event) => this.#apply(event, epoch) });
    } catch {
      // Busy, or the list cannot be read: the one we have stays, and the next read tries again.
    } finally {
      this.#reading = false;
    }
  }

  /** Reads the list now and again every {@link VOLUMES_POLL_MS} until the returned function is called. */
  poll(): () => void {
    void this.refresh();
    const timer = setInterval(() => void this.refresh(), VOLUMES_POLL_MS);
    return () => clearInterval(timer);
  }

  #apply(event: VolumeEvent, epoch: number) {
    if (event.kind === 'list') {
      if (epoch !== this.#epoch) {
        return;
      }

      // A volume keeps the free space measured before until the new measurement arrives, so the menu does not blink.
      // Another device mounted on the same path (another name or kind) is not the one that was measured.
      const known = new Map(this.#list.map((volume) => [volume.path, volume]));
      this.#list = event.volumes.map((volume) => {
        const before = known.get(volume.path);
        const same = before !== undefined && before.name === volume.name && before.kind === volume.kind;
        return { ...volume, free: volume.free ?? (same ? before.free : null) };
      });
    } else {
      this.#list = this.#list.map((volume) => (volume.path === event.path ? { ...volume, free: event.bytes } : volume));
    }
  }
}
