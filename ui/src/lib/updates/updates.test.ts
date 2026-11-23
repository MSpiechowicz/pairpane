import { describe, expect, it, vi } from 'vitest';

import type { UpdateInfo } from '$lib/ipc/bindings/UpdateInfo';
import type { UpdateProgress } from '$lib/ipc/bindings/UpdateProgress';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { Updates } from './updates.svelte';

const info: UpdateInfo = { version: '0.2.0', currentVersion: '0.1.1', date: null, notes: null };

type Handler = (args: unknown) => unknown;

/** A transport whose commands are the given handlers; anything else rejects. */
function transport(handlers: Record<string, Handler>): Transport {
  return {
    invoke: vi.fn(async (name: string, args: unknown) => {
      const handler = handlers[name];
      if (!handler) {
        throw new Error(`unexpected command ${name}`);
      }

      return handler(args);
    }) as unknown as Transport['invoke'],
  };
}

const failing = (code: string) => () => {
  throw new IpcFailure({ code: code as never, message: 'backend detail' });
};

describe('Updates', () => {
  it('offers an update found at startup', async () => {
    const updates = new Updates(transport({ update_check: () => info }));

    await updates.check(false);

    expect(updates.state).toEqual({ kind: 'available', info });
  });

  it('makes the check at startup unless the settings say not to', async () => {
    const check = vi.fn(() => info);
    const wanted = new Updates(transport({ update_check: check }), undefined, undefined, () => true);
    const unwanted = new Updates(transport({ update_check: check }), undefined, undefined, () => false);

    await unwanted.checkAtStart();
    expect(check).not.toHaveBeenCalled();
    expect(unwanted.state).toEqual({ kind: 'idle' });

    await wanted.checkAtStart();
    expect(wanted.state).toEqual({ kind: 'available', info });
  });

  it('a check the user asks for is made whatever the settings say', async () => {
    const updates = new Updates(transport({ update_check: () => info }), undefined, undefined, () => false);

    await updates.check(true);

    expect(updates.state).toEqual({ kind: 'available', info });
  });

  it('stays silent when a startup check finds nothing or fails', async () => {
    const none = new Updates(transport({ update_check: () => null }));
    const offline = new Updates(transport({ update_check: failing('io') }));

    await none.check(false);
    await offline.check(false);

    expect(none.state).toEqual({ kind: 'idle' });
    expect(offline.state).toEqual({ kind: 'idle' });
  });

  it('confirms "up to date" and reports failures for a manual check', async () => {
    const current = new Updates(transport({ update_check: () => null }), () => '0.1.1');
    const offline = new Updates(transport({ update_check: failing('timeout') }));

    await current.check(true);
    await offline.check(true);

    expect(current.state).toEqual({ kind: 'upToDate', version: '0.1.1' });
    expect(offline.state).toMatchObject({ kind: 'failed', message: expect.stringContaining('Could not reach') });
  });

  it('installs only after a found update, streams progress, then is ready to restart', async () => {
    const seen: UpdateProgress[] = [];
    const updates = new Updates(
      transport({
        update_check: () => info,
        update_install: (args) => {
          const { version, onProgress } = args as { version: string; onProgress: (progress: UpdateProgress) => void };
          expect(version).toBe('0.2.0');
          const progress: UpdateProgress = { phase: 'downloading', downloaded: 50, total: 100 };
          onProgress(progress);
          seen.push(progress);

          return null;
        },
      }),
    );

    await updates.install();
    expect(updates.state).toEqual({ kind: 'idle' }); // nothing to install yet

    await updates.check(false);
    await updates.install();

    expect(seen).toHaveLength(1);
    expect(updates.state).toEqual({ kind: 'ready', info });
  });

  it('explains a security failure without the backend detail and allows a retry', async () => {
    const updates = new Updates(transport({ update_check: () => info, update_install: failing('forbidden') }));

    await updates.check(false);
    await updates.install();

    expect(updates.state).toEqual({
      kind: 'failed',
      message: 'The update failed a security check and was not installed.',
      info,
    });
  });

  it('does not relaunch when the guard says no, and does when it says yes', async () => {
    const relaunch = vi.fn(() => null);
    const keep = new Updates(transport({ app_relaunch: relaunch }), undefined, () => Promise.resolve(false));
    const go = new Updates(transport({ app_relaunch: relaunch }), undefined, () => Promise.resolve(true));

    await keep.relaunch();
    expect(relaunch).not.toHaveBeenCalled();

    await go.relaunch();
    expect(relaunch).toHaveBeenCalledOnce();
  });

  it('asks the guard before installing, as the installer may exit the app: no leaves the update on offer', async () => {
    const install = vi.fn(() => null);
    const handlers = { update_check: () => info, update_install: install };
    const keep = new Updates(transport(handlers), undefined, () => Promise.resolve(false));
    const go = new Updates(transport(handlers), undefined, () => Promise.resolve(true));
    await keep.check(false);
    await go.check(false);

    await keep.install();
    expect(install).not.toHaveBeenCalled();
    expect(keep.state).toEqual({ kind: 'available', info });

    await go.install();
    expect(install).toHaveBeenCalledOnce();
    expect(go.state).toEqual({ kind: 'ready', info });
  });

  it('relaunches through the backend', async () => {
    const relaunch = vi.fn(() => null);
    const updates = new Updates(transport({ app_relaunch: relaunch }));

    await updates.relaunch();

    expect(relaunch).toHaveBeenCalledOnce();
  });

  it('ignores a slow startup check once a manual check has started', async () => {
    let finishStartup: (value: UpdateInfo | null) => void = () => {};
    let calls = 0;
    const updates = new Updates(
      transport({
        update_check: () => {
          calls += 1;
          return calls === 1 ? new Promise((resolve) => (finishStartup = resolve)) : info;
        },
      }),
    );

    const startup = updates.check(false);
    await updates.check(true);
    expect(updates.state).toEqual({ kind: 'available', info });

    finishStartup(null);
    await startup;

    expect(updates.state).toEqual({ kind: 'available', info });
  });

  it('does not offer an update again after it was installed in this session', async () => {
    const updates = new Updates(transport({ update_check: () => info, update_install: () => null }), () => '0.1.1');

    await updates.check(false);
    await updates.install();
    updates.dismiss();
    await updates.check(true);

    expect(updates.state).toEqual({ kind: 'upToDate', version: '0.2.0' });
  });
});

