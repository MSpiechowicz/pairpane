import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Volume } from '$lib/ipc/bindings/Volume';
import { IpcFailure } from '$lib/ipc/transport';

import { fakeBackend } from '../../../tests/panes';
import { Volumes, VOLUMES_POLL_MS } from './volumes.svelte';

const volume = (name: string, path: string, free: number | null = null, kind: Volume['kind'] = 'drive'): Volume => ({
  name,
  label: null,
  path,
  kind,
  free,
});

afterEach(() => vi.useRealTimers());

describe('Volumes', () => {
  it('lists the volumes the backend sends, then fills in their free space as it is measured', async () => {
    const backend = fakeBackend();
    const volumes = new Volumes(backend.ipc);

    await volumes.refresh();
    backend.volumes[0].emit({ kind: 'list', volumes: [volume('System', '/'), volume('USB', '/usb')] });
    expect(volumes.list.map((item) => item.free)).toEqual([null, null]);

    backend.volumes[0].emit({ kind: 'free', path: '/usb', bytes: 5000 });
    expect(volumes.list.map((item) => item.free)).toEqual([null, 5000]);
  });

  it('keeps the free space it knows while the next list is read, so the menu does not blink', async () => {
    const backend = fakeBackend();
    const volumes = new Volumes(backend.ipc);
    await volumes.refresh();
    backend.volumes[0].emit({ kind: 'list', volumes: [volume('USB', '/usb')] });
    backend.volumes[0].emit({ kind: 'free', path: '/usb', bytes: 5000 });

    await volumes.refresh();
    backend.volumes[1].emit({ kind: 'list', volumes: [volume('USB', '/usb'), volume('New', '/new')] });

    expect(volumes.list.map((item) => item.free)).toEqual([5000, null]);
  });

  it('does not hand the free space of one device to another that took over its path', async () => {
    const backend = fakeBackend();
    const volumes = new Volumes(backend.ipc);
    await volumes.refresh();
    backend.volumes[0].emit({ kind: 'list', volumes: [volume('USB', '/mnt/disk', null, 'removable')] });
    backend.volumes[0].emit({ kind: 'free', path: '/mnt/disk', bytes: 5000 });

    await volumes.refresh();
    backend.volumes[1].emit({ kind: 'list', volumes: [volume('Backup', '/mnt/disk', null, 'removable')] });
    expect(volumes.list.map((item) => item.free)).toEqual([null]); // another name

    backend.volumes[1].emit({ kind: 'free', path: '/mnt/disk', bytes: 700 });
    await volumes.refresh();
    backend.volumes[2].emit({ kind: 'list', volumes: [volume('Backup', '/mnt/disk', null, 'drive')] });
    expect(volumes.list.map((item) => item.free)).toEqual([null]); // another kind

    backend.volumes[2].emit({ kind: 'free', path: '/mnt/disk', bytes: 900 });
    await volumes.refresh();
    backend.volumes[3].emit({ kind: 'list', volumes: [volume('Backup', '/mnt/disk', null, 'drive')] });
    expect(volumes.list.map((item) => item.free)).toEqual([900]); // the same device
  });

  it('ignores the list of a read that a newer one has overtaken', async () => {
    const backend = fakeBackend();
    const volumes = new Volumes(backend.ipc);
    await volumes.refresh();
    await volumes.refresh();

    backend.volumes[1].emit({ kind: 'list', volumes: [volume('New', '/new')] });
    backend.volumes[0].emit({ kind: 'list', volumes: [volume('Old', '/old')] });

    expect(volumes.list.map((item) => item.name)).toEqual(['New']);
  });

  it('keeps the last list when the backend is busy or fails', async () => {
    let failure: IpcFailure | undefined;
    const backend = fakeBackend({
      volumes_list: () => (failure ? Promise.reject(failure) : null),
    });
    const volumes = new Volumes(backend.ipc);
    await volumes.refresh();
    backend.volumes[0].emit({ kind: 'list', volumes: [volume('System', '/')] });

    failure = new IpcFailure({ code: 'busy', message: 'try again' });
    await volumes.refresh();
    failure = new IpcFailure({ code: 'internal', message: 'broken' });
    await volumes.refresh();

    expect(volumes.list.map((item) => item.name)).toEqual(['System']);
  });

  it('does not start a read while one is still being asked for', async () => {
    const backend = fakeBackend({ volumes_list: () => new Promise(() => {}) });
    const volumes = new Volumes(backend.ipc);

    void volumes.refresh();
    await volumes.refresh();

    expect(backend.volumes).toHaveLength(1);
  });

  it('polls only while asked to: at once, then every 2 s, never after stopping', async () => {
    vi.useFakeTimers();
    const backend = fakeBackend();
    const volumes = new Volumes(backend.ipc);

    const stop = volumes.poll();
    await vi.advanceTimersByTimeAsync(0);
    expect(backend.volumes).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(VOLUMES_POLL_MS * 2);
    expect(backend.volumes).toHaveLength(3);

    stop();
    await vi.advanceTimersByTimeAsync(VOLUMES_POLL_MS * 5);
    expect(backend.volumes).toHaveLength(3);
  });
});
