// Test helpers for the panes: entries and a fake backend whose `pane_open` channel the test drives.
import { vi } from 'vitest';

import type { Entry } from '$lib/ipc/bindings/Entry';
import type { ListEvent } from '$lib/ipc/bindings/ListEvent';
import type { Transport } from '$lib/ipc/transport';

export function entry(name: string, kind: Entry['kind'] = 'file', extra: Partial<Entry> = {}): Entry {
  return { name, kind, size: 0, modified: null, hidden: false, readonly: false, mode: null, linkTarget: null, ...extra };
}

/** A transport that records every `pane_open` and lets the test send events on its channel. */
export function fakeBackend() {
  const opens: { args: Record<string, unknown>; emit: (event: ListEvent) => void }[] = [];
  const ipc = {
    invoke: vi.fn((_name: string, args: Record<string, unknown> & { onEvent: (event: ListEvent) => void }) => {
      opens.push({ args, emit: args.onEvent });
      return Promise.resolve(null);
    }),
  } as unknown as Transport;

  return { ipc, opens, last: () => opens[opens.length - 1] };
}
