// A fake backend for the commands that stream their events on a channel (a search, a comparison): the test drives the channels.
import { vi } from 'vitest';

import type { CommandName } from '$lib/ipc/commands';
import type { Transport } from '$lib/ipc/transport';

/**
 * A transport that remembers the channel of every call of `start` (`started`; `emit` sends an event on it). The calls get the
 * ids 1, 2, ...; `answers` decides what the other commands do (`null` by default); `calls` has the arguments of every call of a command.
 */
export function channelBackend<Args, Event>(start: CommandName, answers: Partial<Record<CommandName, (args: never) => unknown>> = {}) {
  const started: { args: Args; emit: (event: Event) => void }[] = [];
  const invoke = vi.fn(async (name: CommandName, args: { onEvent?: (event: Event) => void }) => {
    if (name === start) {
      started.push({ args: args as never, emit: args.onEvent as never });
      return answers[start]?.(args as never) ?? started.length;
    }

    return answers[name]?.(args as never) ?? null;
  });

  const calls = (name: CommandName) => invoke.mock.calls.filter(([command]) => command === name).map(([, args]) => args);

  return { ipc: { invoke } as unknown as Transport, invoke, started, calls };
}
