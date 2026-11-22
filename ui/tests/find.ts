// Test helpers for Find: hits as the backend describes them, and a fake backend whose search channels the test drives.
import { vi } from 'vitest';

import type { FindEvent } from '$lib/ipc/bindings/FindEvent';
import type { FindHit } from '$lib/ipc/bindings/FindHit';
import type { CommandName } from '$lib/ipc/commands';
import type { Transport } from '$lib/ipc/transport';

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

/**
 * A transport that remembers the channel of every `find_start` (`searches`; `emit` sends an event on it). Searches get the
 * ids 1, 2, ...; `answers` decides what the other commands do (`null` by default); `calls` has the arguments of every call
 * of a command.
 */
export function findBackend(answers: Partial<Record<CommandName, (args: never) => unknown>> = {}) {
  const searches: { args: { query: { root: string; criteria: unknown } }; emit: (event: FindEvent) => void }[] = [];
  const invoke = vi.fn(async (name: CommandName, args: { onEvent?: (event: FindEvent) => void }) => {
    if (name === 'find_start') {
      searches.push({ args: args as never, emit: args.onEvent as never });
      return answers.find_start?.(args as never) ?? searches.length;
    }

    return answers[name]?.(args as never) ?? null;
  });

  const calls = (name: CommandName) => invoke.mock.calls.filter(([command]) => command === name).map(([, args]) => args);

  return { ipc: { invoke } as unknown as Transport, invoke, searches, calls };
}
