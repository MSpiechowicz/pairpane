// Test helpers for Find: hits as the backend describes them, and a fake backend whose search channels the test drives.
import type { FindEvent } from '$lib/ipc/bindings/FindEvent';
import type { FindHit } from '$lib/ipc/bindings/FindHit';
import type { CommandName } from '$lib/ipc/commands';

import { channelBackend } from './channels';

export function hit(path: string, extra: Partial<FindHit> = {}): FindHit {
  return {
    path,
    inner: null,
    kind: 'file',
    link: false,
    size: 2048,
    modified: Date.UTC(2026, 10, 20, 12, 0),
    matches: null,
    line: null,
    lineNumber: null,
    ...extra,
  };
}

export const done = (found: number, extra: Partial<Extract<FindEvent, { kind: 'done' }>> = {}): FindEvent => ({
  kind: 'done',
  found,
  skipped: 0,
  truncated: false,
  cancelled: false,
  ...extra,
});

/** A fake backend whose searches (`find_start`, ids 1, 2, ...) the test drives; see `channelBackend`. */
export function findBackend(answers: Partial<Record<CommandName, (args: never) => unknown>> = {}) {
  const { started, ...backend } = channelBackend<{ query: { root: string; criteria: unknown } }, FindEvent>('find_start', answers);
  return { ...backend, searches: started };
}
