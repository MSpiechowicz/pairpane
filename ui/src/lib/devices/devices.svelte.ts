import type { DevicesEvent } from '$lib/ipc/bindings/DevicesEvent';
import type { DevicesOverview } from '$lib/ipc/bindings/DevicesOverview';
import type { FoundDevice } from '$lib/ipc/bindings/FoundDevice';
import type { PairedDevice } from '$lib/ipc/bindings/PairedDevice';
import type { PairingEnd } from '$lib/ipc/bindings/PairingEnd';
import type { PairingShown } from '$lib/ipc/bindings/PairingShown';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { pairingEndText, type Status } from './format';

/** Which panel of the screen is open under the buttons: the code this device offers, or the form that joins another. */
export type Mode = 'none' | 'pair' | 'join';

export interface Notice {
  text: string;
  /** A refusal, not news. */
  bad: boolean;
}

const STATUS_EVERY = 10_000;
const FOUND_EVERY = 2_000;
const TICK = 1_000;
/** How long a used code may go without the `paired` event that says who used it. */
const PAIRED_WITHIN = 3_000;

let lastSession = 0;

/** A number for each `open()`: strictly increasing, and above those of the pages before a reload (the clock). */
function nextSession(): number {
  lastSession = Math.max(lastSession + 1, Date.now() * 1000);

  return lastSession;
}

/**
 * The Devices screen: this device, the paired ones with their status, the code this device offers and the form that
 * joins another. It is open between `open()` and `close()`; whatever answers or events arrive after that are ignored,
 * and the timers (the countdown, the status checks, the search of the network) stop with it.
 */
export class Devices {
  overview = $state<DevicesOverview>();
  /** Why the screen could not be opened. */
  error = $state<string>();
  mode = $state<Mode>('none');
  offer = $state<PairingShown>();
  /** The milliseconds the offered code has left. */
  remaining = $state(0);
  found = $state<FoundDevice[]>([]);
  statuses = $state<Record<string, Status>>({});
  /** A code is being asked for or a join is on its way. */
  busy = $state(false);
  notice = $state<Notice>();

  readonly #ipc: Transport;
  /** Bumped by every `open()` and `close()`: what began under an older one is no longer wanted. */
  #epoch = 0;
  #deadline = 0;
  #tick: ReturnType<typeof setInterval> | undefined;
  #statusTimer: ReturnType<typeof setInterval> | undefined;
  #foundTimer: ReturnType<typeof setInterval> | undefined;
  #checking = false;
  /** The devices whose status was asked for while a round ran: they are checked as soon as it ends. */
  #queued = new Map<string, PairedDevice>();
  #searching = false;
  /** The withdrawal of the code that is on its way: a new code is asked for only after it. */
  #withdrawal: Promise<void> = Promise.resolve();
  /** This page withdrew a code and the backend's `cancelled` event for it has not come: that one is not news. */
  #expectCancelled = false;
  #usedTimer: ReturnType<typeof setTimeout> | undefined;

  constructor(ipc: Transport) {
    this.#ipc = ipc;
  }

  /** Opens the screen: the backend starts the device link if it has not, and the paired devices are checked. */
  async open(): Promise<void> {
    this.close();
    const epoch = this.#epoch;
    this.error = undefined;

    try {
      const overview = await this.#ipc.invoke('devices_open', {
        onEvent: (event) => this.#onEvent(event, epoch),
        session: nextSession(),
      });
      if (epoch !== this.#epoch) {
        return;
      }

      this.overview = overview;
      this.#startStatusChecks(epoch);
    } catch (error) {
      if (epoch === this.#epoch) {
        this.error = IpcFailure.from(error).message;
      }
    }
  }

  /** Closes the screen: a code that is offered is withdrawn, the timers stop, and nothing that is late is heard. */
  close(): void {
    this.#epoch += 1;
    if (this.offer !== undefined) {
      this.#withdraw();
    }

    this.#stopTimers();
    this.#checking = false;
    this.#queued.clear();
    this.#searching = false;
    this.#expectCancelled = false;
    this.busy = false;
    this.overview = undefined;
    this.error = undefined;
    this.mode = 'none';
    this.offer = undefined;
    this.found = [];
    this.statuses = {};
    this.notice = undefined;
  }

  /** The buttons that open a panel: choosing the one that is open closes it. */
  choose(mode: 'pair' | 'join'): void {
    if (this.mode === mode) {
      this.closePanel();
      return;
    }

    if (this.busy) {
      return;
    }

    this.closePanel();
    this.notice = undefined;
    this.mode = mode;
    if (mode === 'pair') {
      void this.#pair(this.#epoch);
    } else {
      this.#startSearch(this.#epoch);
    }
  }

  /** Closes the open panel; the code it offers is withdrawn. */
  closePanel(): void {
    if (this.offer !== undefined) {
      this.#withdraw();
    }

    this.#finishPanel();
  }

  /** The panel is done: its code (if any) is not shown and its search stops. */
  #finishPanel(): void {
    this.#endOffer();
    this.#stopSearch();
    this.mode = 'none';
  }

  /** Pairs with the device that has `code` on its screen and is found at `target` (an address, or a node id). */
  async join(code: string, target: string): Promise<void> {
    if (this.busy) {
      return;
    }

    const epoch = this.#epoch;
    this.busy = true;
    this.notice = undefined;

    try {
      const device = await this.#ipc.invoke('devices_join', { code: code.trim(), target: target.trim() });
      if (epoch === this.#epoch) {
        this.#upsert(device);
        void this.#checkStatuses(epoch, [device]);
        this.closePanel();
        this.notice = { text: `Paired with ${device.name}.`, bad: false };
      }
    } catch (error) {
      this.#fail(epoch, error);
    } finally {
      if (epoch === this.#epoch) {
        this.busy = false;
      }
    }
  }

  /** Gives the device a name; whether it was done. */
  async rename(nodeId: string, name: string): Promise<boolean> {
    return this.#change(() => this.#ipc.invoke('devices_rename', { nodeId, name }));
  }

  /** Forgets the device; whether it was done. */
  async unpair(nodeId: string): Promise<boolean> {
    return this.#change(() => this.#ipc.invoke('devices_unpair', { nodeId }));
  }

  async #change(send: () => Promise<PairedDevice[]>): Promise<boolean> {
    const epoch = this.#epoch;
    this.notice = undefined;

    try {
      const devices = await send();
      if (epoch === this.#epoch) {
        this.#replace(devices);
      }

      return true;
    } catch (error) {
      this.#fail(epoch, error);

      return false;
    }
  }

  async #pair(epoch: number): Promise<void> {
    this.busy = true;

    try {
      // A code that was withdrawn a moment ago is gone before the next one is asked for.
      await this.#withdrawal;
      if (epoch !== this.#epoch || this.mode !== 'pair') {
        return;
      }

      const shown = await this.#ipc.invoke('devices_pair_begin', undefined);
      if (epoch !== this.#epoch || this.mode !== 'pair') {
        // The screen (or the panel) went away while the code was being made: the code goes with it.
        this.#withdraw(epoch);
        return;
      }

      this.offer = shown;
      this.#deadline = Date.now() + shown.expiresInMs;
      this.remaining = shown.expiresInMs;
      this.#tick = setInterval(() => this.#count(epoch), TICK);
    } catch (error) {
      if (epoch === this.#epoch) {
        this.mode = 'none';
      }

      this.#fail(epoch, error);
    } finally {
      if (epoch === this.#epoch) {
        this.busy = false;
      }
    }
  }

  #count(epoch: number): void {
    if (epoch !== this.#epoch || this.offer === undefined) {
      return;
    }

    this.remaining = Math.max(0, this.#deadline - Date.now());
    if (this.remaining === 0) {
      this.#endWith(pairingEndText('expired'));
    }
  }

  /** The code is not offered any more (the answer of the backend does not matter: the page is done with it). */
  #withdraw(epoch = this.#epoch): void {
    // Only the page of the screen that is open hears the event; the one of a screen that is gone is not waited for.
    const expected = epoch === this.#epoch;
    this.#expectCancelled = expected;
    const withdrawal = this.#ipc.invoke('devices_pair_cancel', undefined).then(
      (ended) => {
        // No code was offered any more (it had expired or been used): no `cancelled` event comes, so none is waited for.
        if (ended === false && expected && this.#withdrawal === withdrawal) {
          this.#expectCancelled = false;
        }
      },
      () => {},
    );
    this.#withdrawal = withdrawal;
  }

  #endOffer(): void {
    this.offer = undefined;
    clearInterval(this.#tick);
    clearTimeout(this.#usedTimer);
    this.#tick = undefined;
    this.#usedTimer = undefined;
  }

  /** The offer is over and the panel with it; `text` says why. */
  #endWith(text: string): void {
    this.#finishPanel();
    this.notice = { text, bad: false };
  }

  #onEvent(event: DevicesEvent, epoch: number): void {
    if (epoch !== this.#epoch) {
      return;
    }

    switch (event.kind) {
      case 'paired':
        this.#upsert(event.device);
        void this.#checkStatuses(epoch, [event.device]);
        if (this.offer !== undefined) {
          this.#endWith(`Paired with ${event.device.name}.`);
        }
        break;
      case 'unpaired':
        this.#replace(this.overview?.devices.filter((device) => device.nodeId !== event.nodeId) ?? []);
        break;
      case 'pairingEnded':
        this.#onPairingEnded(event.reason);
        break;
      case 'resync':
        // The events that came while the link was not heard are lost, the end of the offer may be one of them.
        this.#replace(event.devices);
        if (this.offer !== undefined) {
          this.closePanel();
        }
        break;
    }
  }

  #onPairingEnded(reason: PairingEnd): void {
    if (reason === 'cancelled' && this.#expectCancelled) {
      // The echo of this page's own withdrawal.
      this.#expectCancelled = false;
      return;
    }

    if (this.offer === undefined) {
      return;
    }

    if (reason === 'used') {
      // The `paired` event says who; when it does not come, the user is told instead of left with a code that is dead.
      this.#usedTimer ??= setTimeout(() => this.#endWith('The code was used, but pairing did not finish. Try again.'), PAIRED_WITHIN);
    } else {
      this.#endWith(pairingEndText(reason));
    }
  }

  #fail(epoch: number, error: unknown): void {
    if (epoch === this.#epoch) {
      this.notice = { text: IpcFailure.from(error).message, bad: true };
    }
  }

  #upsert(device: PairedDevice): void {
    const devices = this.overview?.devices ?? [];
    const known = devices.some((one) => one.nodeId === device.nodeId);
    this.#replace(known ? devices.map((one) => (one.nodeId === device.nodeId ? device : one)) : [...devices, device]);
  }

  /** The list of paired devices is what the backend says; a device that is gone loses its status. */
  #replace(devices: PairedDevice[]): void {
    if (this.overview === undefined) {
      return;
    }

    this.overview = { ...this.overview, devices };
    const present = new Set(devices.map((device) => device.nodeId));
    this.statuses = Object.fromEntries(Object.entries(this.statuses).filter(([nodeId]) => present.has(nodeId)));
  }

  #startStatusChecks(epoch: number): void {
    void this.#checkStatuses(epoch, this.overview?.devices ?? [], false);
    this.#statusTimer = setInterval(() => void this.#checkStatuses(epoch, this.overview?.devices ?? [], false), STATUS_EVERY);
  }

  /**
   * Asks the backend whether the devices are reachable; one round at a time, none while the page is hidden. A device
   * that is new (`wait`) is not left for the next round of the timer: it is checked right after the one that runs.
   */
  async #checkStatuses(epoch: number, devices: readonly PairedDevice[], wait = true): Promise<void> {
    if (document.hidden || devices.length === 0) {
      return;
    }

    if (this.#checking) {
      if (wait) {
        devices.forEach((device) => this.#queued.set(device.nodeId, device));
      }

      return;
    }

    this.#checking = true;
    const answers = await Promise.allSettled(devices.map((device) => this.#ipc.invoke('devices_status', { nodeId: device.nodeId })));
    if (epoch !== this.#epoch) {
      return;
    }

    this.#checking = false;
    const present = new Set(this.overview?.devices.map((device) => device.nodeId));
    const statuses = { ...this.statuses };
    answers.forEach((answer, index) => {
      const { nodeId } = devices[index];
      if (present.has(nodeId)) {
        statuses[nodeId] = answer.status === 'fulfilled' ? answer.value : { kind: 'offline' };
      }
    });
    this.statuses = statuses;

    const next = [...this.#queued.values()];
    this.#queued.clear();
    await this.#checkStatuses(epoch, next);
  }

  #startSearch(epoch: number): void {
    void this.#search(epoch);
    this.#foundTimer = setInterval(() => void this.#search(epoch), FOUND_EVERY);
  }

  async #search(epoch: number): Promise<void> {
    if (this.#searching) {
      return;
    }

    this.#searching = true;
    try {
      const found = await this.#ipc.invoke('devices_discovered', undefined);
      if (epoch === this.#epoch && this.mode === 'join') {
        this.found = found;
      }
    } catch {
      // The search is a help: the target can be typed.
    } finally {
      if (epoch === this.#epoch) {
        this.#searching = false;
      }
    }
  }

  #stopSearch(): void {
    clearInterval(this.#foundTimer);
    this.#foundTimer = undefined;
    this.#searching = false;
    this.found = [];
  }

  #stopTimers(): void {
    clearInterval(this.#tick);
    clearInterval(this.#statusTimer);
    this.#tick = undefined;
    this.#statusTimer = undefined;
    this.#stopSearch();
  }
}
