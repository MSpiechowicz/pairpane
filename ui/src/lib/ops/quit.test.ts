import { render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { describe, expect, it } from 'vitest';

import { Unsaved } from '$lib/editor/unsaved';
import { Preferences } from '$lib/shell/preferences.svelte';

import { job, opsBackend } from '../../../tests/ops';
import WatchHost from '../../../tests/WatchHost.svelte';
import { QuitGuard } from './quit.svelte';
import { Transfers } from './transfers.svelte';

async function world(set: Parameters<Preferences['set']>[0] = {}) {
  const backend = opsBackend();
  const transfers = new Transfers(backend.ipc);
  await transfers.subscribe();
  const preferences = new Preferences();
  preferences.set(set);
  const unsaved = new Unsaved(backend.ipc);
  const guard = new QuitGuard(unsaved, transfers, preferences);
  render(WatchHost, { start: () => guard.watch() });

  const told = () => backend.calls('editor_set_dirty');
  const start = () => (backend.send({ kind: 'added', job: job(1) }), flushSync());
  const end = () => {
    backend.send({ kind: 'ended', id: 1, outcome: { kind: 'done' }, skipped: 0, errors: [], moreErrors: 0 });
    flushSync();
  };

  return { guard, preferences, unsaved, told, start, end };
}

describe('QuitGuard', () => {
  it('makes a close worth asking about while transfers run, and not before or after', async () => {
    const { told, start, end, unsaved } = await world();
    expect(told()).toEqual([]);

    start();
    expect(unsaved.dirty).toBe(true);
    end();

    expect(told()).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('says nothing when the setting is off, and follows it when it changes while transfers run', async () => {
    const { told, start, preferences } = await world({ confirmQuitWithTransfers: false });

    start();
    expect(told()).toEqual([]);

    preferences.set({ confirmQuitWithTransfers: true });
    flushSync();
    expect(told()).toEqual([{ dirty: true }]);

    preferences.set({ confirmQuitWithTransfers: false });
    flushSync();
    expect(told()).toEqual([{ dirty: true }, { dirty: false }]);
  });

  it('lets the app end at once with no transfer running, or with the setting off', async () => {
    const idle = await world();
    expect(await idle.guard.discard()).toBe('go');
    expect(idle.guard.asking).toBe(false);

    const off = await world({ confirmQuitWithTransfers: false });
    off.start();
    expect(await off.guard.discard()).toBe('go');
    expect(off.guard.asking).toBe(false);
  });

  it('asks while transfers run: Quit lets the app end, Cancel keeps it', async () => {
    const { guard, start } = await world();
    start();

    const quitting = guard.discard();
    expect(guard.asking).toBe(true);
    guard.answer(true);
    expect(await quitting).toBe('go');
    expect(guard.asking).toBe(false);

    const staying = guard.discard();
    guard.answer(false);
    expect(await staying).toBe('stay');
    expect(guard.asking).toBe(false);
  });

  it('answers a question replaced by a newer one as replaced, so that only the newer one acts', async () => {
    const { guard, start } = await world();
    start();

    const first = guard.discard();
    const second = guard.discard();
    expect(await first).toBe('replaced');
    guard.answer(true);

    expect(await second).toBe('go');
  });

  it('asks only after an earlier question let the close go on', async () => {
    const { guard, start } = await world();
    start();

    expect(await guard.after('stay')).toBe('stay');
    expect(await guard.after('replaced')).toBe('replaced');
    expect(guard.asking).toBe(false);

    const next = guard.after('go');
    expect(guard.asking).toBe(true);
    guard.answer(false);
    expect(await next).toBe('stay');
  });

  it('lets the close go on by itself when the last transfer ends while the question is on the screen', async () => {
    const { guard, start, end } = await world();
    start();
    const answer = guard.discard();
    expect(guard.asking).toBe(true);

    end();

    expect(await answer).toBe('go');
    expect(guard.asking).toBe(false);
  });
});
