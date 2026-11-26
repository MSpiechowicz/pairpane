import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { IpcFailure } from '$lib/ipc/transport';

import { devicesBackend, offered, ONE, overviewOf, paired, TWO } from '../../../tests/devices';
import { Devices } from './devices.svelte';

const laptop = paired(ONE, 'Laptop');
const phone = paired(TWO, 'Phone');

async function opened(devices = [laptop], answers: Parameters<typeof devicesBackend>[1] = {}) {
  const backend = devicesBackend(devices, answers);
  const screen = new Devices(backend.ipc);
  await screen.open();

  return { backend, screen };
}

const settle = () => vi.advanceTimersByTimeAsync(0);

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('opening', () => {
  it('shows the overview, registers its channel and checks the status of each device', async () => {
    const { backend, screen } = await opened([laptop, phone]);
    await settle();

    expect(screen.overview?.devices).toEqual([laptop, phone]);
    expect(backend.started).toHaveLength(1);
    expect(backend.calls('devices_status')).toEqual([{ nodeId: ONE }, { nodeId: TWO }]);
    expect(screen.statuses[ONE]).toEqual({ kind: 'online', name: 'Laptop' });
  });

  it('sends a session that is higher with every opening, so a late one cannot take the channel back', async () => {
    const { backend, screen } = await opened([]);
    await screen.open();
    screen.close();
    await screen.open();

    const sessions = backend.started.map(({ args }) => args.session);
    expect(sessions).toHaveLength(3);
    expect(sessions[1]).toBeGreaterThan(sessions[0]);
    expect(sessions[2]).toBeGreaterThan(sessions[1]);
  });

  it('keeps the message of a refusal and opens again on request', async () => {
    let fail = true;
    const { screen } = await opened([], {
      devices_open: () => (fail ? Promise.reject(new IpcFailure({ code: 'internal', message: 'The device link is not running.' })) : overviewOf()),
    });
    expect(screen.error).toBe('The device link is not running.');
    expect(screen.overview).toBeUndefined();

    fail = false;
    await screen.open();

    expect(screen.error).toBeUndefined();
  });
});

describe('the status checks', () => {
  it('repeat every ten seconds, never overlap, and wait while the page is hidden', async () => {
    let answer: (status: unknown) => void = () => {};
    let first = true;
    const { backend, screen } = await opened([laptop], {
      devices_status: () => {
        if (!first) {
          return { kind: 'offline' };
        }

        first = false;
        return new Promise((resolve) => (answer = resolve));
      },
    });
    expect(backend.count('devices_status')).toBe(1);

    await vi.advanceTimersByTimeAsync(30_000);
    expect(backend.count('devices_status')).toBe(1); // the first round is still out

    answer({ kind: 'offline' });
    await vi.advanceTimersByTimeAsync(10_000);
    expect(backend.count('devices_status')).toBe(2);
    expect(screen.statuses[ONE]).toEqual({ kind: 'offline' });

    const hidden = vi.spyOn(document, 'hidden', 'get').mockReturnValue(true);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(backend.count('devices_status')).toBe(2);

    hidden.mockReturnValue(false);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(backend.count('devices_status')).toBe(3);
    hidden.mockRestore();
  });

  it('check a device that pairs while a round runs right after the round, not at the next tick', async () => {
    let answer: (status: unknown) => void = () => {};
    let first = true;
    const { backend, screen } = await opened([laptop], {
      devices_status: () => {
        if (!first) {
          return { kind: 'online', name: 'Phone' };
        }

        first = false;
        return new Promise((resolve) => (answer = resolve));
      },
    });

    backend.emit({ kind: 'paired', device: phone });
    await settle();
    expect(backend.calls('devices_status')).toEqual([{ nodeId: ONE }]); // the first round is still out

    answer({ kind: 'offline' });
    await settle();

    expect(backend.calls('devices_status')).toEqual([{ nodeId: ONE }, { nodeId: TWO }]);
    expect(screen.statuses[TWO]).toEqual({ kind: 'online', name: 'Phone' });
  });

  it('count a device that fails to answer as offline, and the others still count', async () => {
    const { screen } = await opened([laptop, phone], {
      devices_status: (args: { nodeId: string }) => (args.nodeId === ONE ? Promise.reject(new Error('x')) : { kind: 'refused' }),
    });
    await settle();

    expect(screen.statuses).toEqual({ [ONE]: { kind: 'offline' }, [TWO]: { kind: 'refused' } });
  });
});

describe('offering a code', () => {
  it('asks for one, counts it down once a second and withdraws it on Cancel without a notice', async () => {
    const { backend, screen } = await opened([]);

    screen.choose('pair');
    await settle();
    expect(screen.offer?.code).toBe('482913');
    expect(screen.remaining).toBe(90_000);

    await vi.advanceTimersByTimeAsync(3_000);
    expect(screen.remaining).toBe(87_000);

    screen.closePanel();
    expect(backend.count('devices_pair_cancel')).toBe(1);
    backend.emit({ kind: 'pairingEnded', reason: 'cancelled' });

    expect(screen.offer).toBeUndefined();
    expect(screen.mode).toBe('none');
    expect(screen.notice).toBeUndefined();
    await vi.advanceTimersByTimeAsync(5_000);
    expect(screen.remaining).toBe(87_000); // the countdown stopped
  });

  it('ends with a notice when the backend says the code expired or was tried too often', async () => {
    const { backend, screen } = await opened([]);
    screen.choose('pair');
    await settle();

    backend.emit({ kind: 'pairingEnded', reason: 'tooManyAttempts' });

    expect(screen.offer).toBeUndefined();
    expect(screen.notice).toEqual({ text: expect.stringMatching(/Too many/), bad: false });
  });

  it('ends with a notice when the code was withdrawn by something else than this page (the tray hides it)', async () => {
    const { backend, screen } = await opened([]);
    screen.choose('pair');
    await settle();

    backend.emit({ kind: 'pairingEnded', reason: 'cancelled' });

    expect(screen.offer).toBeUndefined();
    expect(screen.mode).toBe('none');
    expect(screen.notice).toEqual({ text: 'The code was withdrawn.', bad: false });
    expect(backend.count('devices_pair_cancel')).toBe(0);
  });

  it('keeps a new code when the echo of its own withdrawal of the old one comes late', async () => {
    const { backend, screen } = await opened([]);
    screen.choose('pair');
    await settle();

    screen.closePanel();
    screen.choose('pair');
    await settle();
    backend.emit({ kind: 'pairingEnded', reason: 'cancelled' });

    expect(screen.offer).toBeDefined();
    expect(screen.notice).toBeUndefined();
  });

  it('ignores the end of a code it never showed (the one of the page before a reload)', async () => {
    const { backend, screen } = await opened([]);

    backend.emit({ kind: 'pairingEnded', reason: 'cancelled' });
    screen.choose('pair');
    await settle();

    expect(screen.offer).toBeDefined();
    expect(screen.notice).toBeUndefined();
  });

  it('waits for no echo when its withdrawal found no code, so a later withdrawal by the tray still ends the new one', async () => {
    const { backend, screen } = await opened([], { devices_pair_cancel: () => false });
    screen.choose('pair');
    await settle();

    screen.closePanel();
    screen.choose('pair');
    await settle();
    backend.emit({ kind: 'pairingEnded', reason: 'cancelled' });

    expect(screen.offer).toBeUndefined();
    expect(screen.notice).toEqual({ text: 'The code was withdrawn.', bad: false });
  });

  it('asks for a new code only after the old one is withdrawn', async () => {
    let withdrawn: (value: null) => void = () => {};
    const { backend, screen } = await opened([], { devices_pair_cancel: () => new Promise((resolve) => (withdrawn = resolve)) });
    screen.choose('pair');
    await settle();

    screen.closePanel();
    screen.choose('pair');
    await settle();
    expect(backend.count('devices_pair_begin')).toBe(1);

    withdrawn(null);
    await settle();

    expect(backend.count('devices_pair_begin')).toBe(2);
    expect(screen.offer).toBeDefined();
  });

  it('tells the user when a used code is not followed by the pairing within three seconds', async () => {
    const { backend, screen } = await opened([]);
    screen.choose('pair');
    await settle();

    backend.emit({ kind: 'pairingEnded', reason: 'used' });
    await vi.advanceTimersByTimeAsync(2_900);
    expect(screen.offer).toBeDefined();

    await vi.advanceTimersByTimeAsync(200);

    expect(screen.offer).toBeUndefined();
    expect(screen.mode).toBe('none');
    expect(screen.notice).toEqual({ text: 'The code was used, but pairing did not finish. Try again.', bad: false });
  });

  it('a device that pairs after the code was used cancels the wait and is not withdrawn', async () => {
    const { backend, screen } = await opened([]);
    screen.choose('pair');
    await settle();

    backend.emit({ kind: 'pairingEnded', reason: 'used' });
    backend.emit({ kind: 'paired', device: phone });
    await vi.advanceTimersByTimeAsync(5_000);

    expect(screen.notice).toEqual({ text: 'Paired with Phone.', bad: false });
    expect(backend.count('devices_pair_cancel')).toBe(0);
  });

  it('drops the offer when the list is resynced, since its end may have been missed', async () => {
    const { backend, screen } = await opened([]);
    screen.choose('pair');
    await settle();

    backend.emit({ kind: 'resync', devices: [phone] });

    expect(screen.overview?.devices).toEqual([phone]);
    expect(screen.offer).toBeUndefined();
    expect(screen.mode).toBe('none');
    expect(screen.notice).toBeUndefined();
  });

  it('ends by itself when the time is up, in case the event never comes', async () => {
    const { screen } = await opened([], { devices_pair_begin: () => offered({ expiresInMs: 3_000 }) });
    screen.choose('pair');
    await settle();

    await vi.advanceTimersByTimeAsync(3_000);

    expect(screen.offer).toBeUndefined();
    expect(screen.notice?.text).toMatch(/expired/);
  });

  it('a device that pairs with it is added, checked, and told in a notice', async () => {
    const { backend, screen } = await opened([]);
    screen.choose('pair');
    await settle();

    backend.emit({ kind: 'pairingEnded', reason: 'used' });
    expect(screen.offer).toBeDefined(); // the `paired` event is what ends it
    backend.emit({ kind: 'paired', device: phone });
    await settle();

    expect(screen.overview?.devices).toEqual([phone]);
    expect(backend.calls('devices_status')).toContainEqual({ nodeId: TWO });
    expect(screen.offer).toBeUndefined();
    expect(screen.notice).toEqual({ text: 'Paired with Phone.', bad: false });
  });

  it('tells why no code could be made', async () => {
    const { screen } = await opened([], { devices_pair_begin: () => Promise.reject(new IpcFailure({ code: 'busy', message: 'Already offering.' })) });

    screen.choose('pair');
    await settle();

    expect(screen.mode).toBe('none');
    expect(screen.notice).toEqual({ text: 'Already offering.', bad: true });
    expect(screen.busy).toBe(false);
  });

  it('a code made after the panel was closed is withdrawn at once', async () => {
    let make: (shown: unknown) => void = () => {};
    const { backend, screen } = await opened([], { devices_pair_begin: () => new Promise((resolve) => (make = resolve)) });
    screen.choose('pair');
    await settle();
    screen.closePanel();

    make(offered());
    await settle();

    expect(screen.offer).toBeUndefined();
    expect(backend.count('devices_pair_cancel')).toBe(1);
  });

  it('asks for no code when the panel is closed before the old one is withdrawn', async () => {
    const { backend, screen } = await opened([]);

    screen.choose('pair');
    screen.closePanel();
    await settle();

    expect(backend.count('devices_pair_begin')).toBe(0);
    expect(screen.busy).toBe(false);
  });
});

describe('joining', () => {
  it('searches the network every two seconds while the form is open, one search at a time', async () => {
    const found = [{ nodeId: TWO, name: 'Phone', paired: false }];
    const { backend, screen } = await opened([], { devices_discovered: () => found });

    screen.choose('join');
    await settle();
    expect(screen.found).toEqual(found);
    await vi.advanceTimersByTimeAsync(4_000);
    expect(backend.count('devices_discovered')).toBe(3);

    screen.closePanel();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(backend.count('devices_discovered')).toBe(3);
    expect(screen.found).toEqual([]);
  });

  it('sends the trimmed code and target, then shows the new device with a notice', async () => {
    const { backend, screen } = await opened([laptop], { devices_join: () => phone });
    screen.choose('join');

    await screen.join(' 123456 ', ` ${TWO} `);

    expect(backend.calls('devices_join')).toEqual([{ code: '123456', target: TWO }]);
    expect(screen.overview?.devices).toEqual([laptop, phone]);
    expect(screen.mode).toBe('none');
    expect(screen.notice).toEqual({ text: 'Paired with Phone.', bad: false });
    expect(screen.busy).toBe(false);
  });

  it('is busy while it is on its way, takes no second join, and keeps the form when refused', async () => {
    let answer: (error: unknown) => void = () => {};
    const { backend, screen } = await opened([], { devices_join: () => new Promise((_, reject) => (answer = reject)) });
    screen.choose('join');

    const first = screen.join('1', 'a');
    void screen.join('1', 'a');
    expect(screen.busy).toBe(true);
    answer(new IpcFailure({ code: 'io', message: 'The other device could not be reached.' }));
    await first;

    expect(backend.count('devices_join')).toBe(1);
    expect(screen.busy).toBe(false);
    expect(screen.mode).toBe('join');
    expect(screen.notice).toEqual({ text: 'The other device could not be reached.', bad: true });
  });
});

describe('the list', () => {
  it('takes the list the backend answers to a rename and to an unpair', async () => {
    const { backend, screen } = await opened([laptop, phone], {
      devices_rename: () => [paired(ONE, 'Work'), phone],
      devices_unpair: () => [phone],
    });
    await settle();

    expect(await screen.rename(ONE, 'Work')).toBe(true);
    expect(backend.calls('devices_rename')).toEqual([{ nodeId: ONE, name: 'Work' }]);
    expect(screen.overview?.devices[0].name).toBe('Work');

    expect(await screen.unpair(ONE)).toBe(true);
    expect(screen.overview?.devices).toEqual([phone]);
    expect(screen.statuses[ONE]).toBeUndefined();
  });

  it('says when a change is refused, and the list stays', async () => {
    const { screen } = await opened([laptop], { devices_rename: () => Promise.reject(new IpcFailure({ code: 'invalidInput', message: 'That name is not valid.' })) });

    expect(await screen.rename(ONE, '')).toBe(false);

    expect(screen.notice).toEqual({ text: 'That name is not valid.', bad: true });
    expect(screen.overview?.devices).toEqual([laptop]);
  });

  it('follows the events: unpaired removes, resync replaces, paired updates a known device', async () => {
    const { backend, screen } = await opened([laptop, phone]);

    backend.emit({ kind: 'unpaired', nodeId: ONE });
    expect(screen.overview?.devices).toEqual([phone]);

    backend.emit({ kind: 'resync', devices: [laptop] });
    expect(screen.overview?.devices).toEqual([laptop]);

    backend.emit({ kind: 'paired', device: paired(ONE, 'Renamed') });
    expect(screen.overview?.devices).toEqual([paired(ONE, 'Renamed')]);
  });
});

describe('closing', () => {
  it('withdraws a code that is offered, stops every timer and ignores what is late', async () => {
    const { backend, screen } = await opened([laptop]);
    screen.choose('pair');
    await settle();
    const statuses = backend.count('devices_status');

    screen.close();
    expect(backend.count('devices_pair_cancel')).toBe(1);
    backend.emit({ kind: 'paired', device: phone });
    await vi.advanceTimersByTimeAsync(60_000);

    expect(screen.overview).toBeUndefined();
    expect(screen.notice).toBeUndefined();
    expect(backend.count('devices_status')).toBe(statuses);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('does not cancel when no code is offered', async () => {
    const { backend, screen } = await opened([]);

    screen.close();

    expect(backend.count('devices_pair_cancel')).toBe(0);
  });

  it('ignores an overview that comes after it', async () => {
    let answer: (overview: unknown) => void = () => {};
    const backend = devicesBackend([], { devices_open: () => new Promise((resolve) => (answer = resolve)) });
    const screen = new Devices(backend.ipc);
    const opening = screen.open();

    screen.close();
    answer({ own: { nodeId: 'x', name: 'Desk', keyStorage: 'file' }, devices: [laptop] });
    await opening;

    expect(screen.overview).toBeUndefined();
    expect(vi.getTimerCount()).toBe(0);
  });
});
