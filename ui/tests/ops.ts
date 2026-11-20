// Test helpers for copy and move: jobs and questions as the backend describes them, and a fake backend whose
// transfers channel the test drives.
import { vi } from 'vitest';

import type { ConflictInfo } from '$lib/ipc/bindings/ConflictInfo';
import type { JobView } from '$lib/ipc/bindings/JobView';
import type { OpsEvent } from '$lib/ipc/bindings/OpsEvent';
import type { CommandName } from '$lib/ipc/commands';
import type { Transport } from '$lib/ipc/transport';

/** A job that has just been added: copying `a.pdf` into `/to`. */
export function job(id: number, extra: Partial<JobView> = {}): JobView {
  return {
    id,
    kind: 'copy',
    label: 'a.pdf',
    from: '/from',
    to: '/to',
    items: 1,
    verify: false,
    phase: 'copying',
    paused: false,
    doneBytes: 0,
    totalBytes: 0,
    doneFiles: 0,
    totalFiles: 0,
    current: null,
    bytesPerSec: 0,
    etaSecs: null,
    ...extra,
  };
}

/** The target of `a.pdf` exists; the source is the newer one. */
export function conflict(jobId: number, question = 1, extra: Partial<ConflictInfo> = {}): ConflictInfo {
  return {
    job: jobId,
    question,
    name: 'a.pdf',
    source: { size: 4096, modified: Date.UTC(2026, 10, 20, 12, 0), dir: false },
    target: { size: 2048, modified: Date.UTC(2026, 10, 19, 12, 0), dir: false },
    suggested: 'a (2).pdf',
    ...extra,
  };
}

/**
 * A transport that remembers the transfers channel (`send` delivers an event on it). A command is answered with
 * `null`, or as `answers` says (a function of the arguments; an async one may reject); `invoke` has every call.
 */
export function opsBackend(answers: Partial<Record<CommandName, (args: never) => unknown>> = {}) {
  let sink: (event: OpsEvent) => void = () => {};
  const invoke = vi.fn(async (name: CommandName, args: { onEvent?: (event: OpsEvent) => void }) => {
    if (name === 'ops_subscribe' && args.onEvent) {
      sink = args.onEvent;
    }

    return answers[name]?.(args as never) ?? null;
  });

  return {
    ipc: { invoke } as unknown as Transport,
    invoke,
    send: (...events: OpsEvent[]) => events.forEach((event) => sink(event)),
    /** The arguments of every call of one command. */
    calls: (name: CommandName) => invoke.mock.calls.filter(([called]) => called === name).map(([, args]) => args),
  };
}
