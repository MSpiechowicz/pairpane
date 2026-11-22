// Test helpers for Compare and Synchronize: rows as the backend describes them, and a fake backend whose scan channels the test drives.
import type { CompareEvent } from '$lib/ipc/bindings/CompareEvent';
import type { CompareFacts } from '$lib/ipc/bindings/CompareFacts';
import type { CompareOptions } from '$lib/ipc/bindings/CompareOptions';
import type { CompareRoot } from '$lib/ipc/bindings/CompareRoot';
import type { CompareRow } from '$lib/ipc/bindings/CompareRow';
import type { CompareStatus } from '$lib/ipc/bindings/CompareStatus';
import type { CommandName } from '$lib/ipc/commands';

import { channelBackend } from './channels';

export const MODIFIED = Date.UTC(2026, 10, 20, 12, 0);

export const facts = (extra: Partial<CompareFacts> = {}): CompareFacts => ({
  kind: 'file',
  link: false,
  size: 2048,
  modified: MODIFIED,
  files: 0,
  bytes: 0,
  ...extra,
});

/** A row; the sides that are `undefined` hold a file as `facts()` says, `null` is a side that has nothing. */
export function row(path: string, status: CompareStatus, left: CompareFacts | null = facts(), right: CompareFacts | null = facts()): CompareRow {
  return { path, status, left, right };
}

export const rows = (...found: CompareRow[]): CompareEvent => ({ kind: 'rows', rows: found });

export const finished = (count: number, extra: Partial<Extract<CompareEvent, { kind: 'done' }>> = {}): CompareEvent => ({
  kind: 'done',
  rows: count,
  skipped: 0,
  truncated: false,
  cancelled: false,
  ...extra,
});

export const folder = (path: string): CompareRoot => ({ kind: 'folder', path });

/** A fake backend whose scans (`compare_start`, ids 1, 2, ...) the test drives; see `channelBackend`. */
export function compareBackend(answers: Partial<Record<CommandName, (args: never) => unknown>> = {}) {
  const { started, ...backend } = channelBackend<{ left: CompareRoot; right: CompareRoot; options: CompareOptions }, CompareEvent>('compare_start', answers);
  return { ...backend, scans: started };
}
