// Test helpers for the panes: entries and a fake backend whose channels the test drives.
import { vi } from 'vitest';

import type { Entry } from '$lib/ipc/bindings/Entry';
import type { ListEvent } from '$lib/ipc/bindings/ListEvent';
import type { SizeEvent } from '$lib/ipc/bindings/SizeEvent';
import type { Transport } from '$lib/ipc/transport';

export function entry(name: string, kind: Entry['kind'] = 'file', extra: Partial<Entry> = {}): Entry {
  return { name, kind, size: 0, modified: null, hidden: false, readonly: false, mode: null, linkTarget: null, ...extra };
}

/**
 * A transport that records every `pane_open` and `pane_folder_size` (`sizes`) and lets the test send events on
 * their channels.
 */
export function fakeBackend() {
  const opens: { args: Record<string, unknown>; emit: (event: ListEvent) => void }[] = [];
  const sizes: { args: Record<string, unknown>; emit: (event: SizeEvent) => void }[] = [];
  const ipc = {
    invoke: vi.fn((name: string, args: Record<string, unknown> & { onEvent: (event: never) => void }) => {
      (name === 'pane_folder_size' ? sizes : opens).push({ args, emit: args.onEvent } as never);
      return Promise.resolve(null);
    }),
  } as unknown as Transport;

  return { ipc, opens, sizes, last: () => opens[opens.length - 1] };
}

/** Calls `open`, then delivers a whole folder on its channel: `start`, one batch with `entries`, `done`. */
export async function deliver(
  backend: ReturnType<typeof fakeBackend>,
  open: () => Promise<void>,
  path: string,
  entries: Entry[],
  free: number | null = null,
) {
  await open();
  const channel = backend.last();
  channel.emit({
    kind: 'start',
    path,
    parent: path === '/' ? null : '/',
    crumbs: [
      { name: '/', path: '/' },
      { name: path.split('/').pop() ?? '', path },
    ],
    total: entries.length,
  });
  channel.emit({ kind: 'batch', offset: 0, entries });
  channel.emit({
    kind: 'done',
    dirs: entries.filter((e) => e.kind === 'dir').length,
    files: entries.filter((e) => e.kind !== 'dir').length,
    bytes: entries.reduce((sum, e) => sum + e.size, 0),
    free,
  });
  return channel;
}
