// Test helpers for the panes: entries and a fake backend whose channels the test drives.
import { vi } from 'vitest';

import type { Entry } from '$lib/ipc/bindings/Entry';
import type { ArchiveSpot } from '$lib/ipc/bindings/ArchiveSpot';
import type { Crumb } from '$lib/ipc/bindings/Crumb';
import type { FindId } from '$lib/ipc/bindings/FindId';
import type { ListEvent } from '$lib/ipc/bindings/ListEvent';
import type { SizeEvent } from '$lib/ipc/bindings/SizeEvent';
import type { VolumeEvent } from '$lib/ipc/bindings/VolumeEvent';
import type { CommandName } from '$lib/ipc/commands';
import type { Transport } from '$lib/ipc/transport';

export function entry(name: string, kind: Entry['kind'] = 'file', extra: Partial<Entry> = {}): Entry {
  return {
    name,
    kind,
    size: 0,
    modified: null,
    hidden: false,
    readonly: false,
    mode: null,
    linkTarget: null,
    ...extra,
  };
}

type Call = { args: Record<string, unknown>; emit: (event: never) => void };

/**
 * A transport that records every `pane_open`, `archive_open` and `find_open` (`opens`), `pane_folder_size` (`sizes`) and `volumes_list`
 * (`volumes`) and lets the test send events on their channels. A command is answered with `null`, or as `answers`
 * says (a function of the arguments; it may return a promise, also a rejected or a never settled one);
 * `invoke` has the calls of all of them.
 */
export function fakeBackend(answers: Partial<Record<CommandName, (args: never) => unknown>> = {}) {
  const opens: { args: Record<string, unknown>; emit: (event: ListEvent) => void }[] = [];
  const sizes: { args: Record<string, unknown>; emit: (event: SizeEvent) => void }[] = [];
  const volumes: { args: Record<string, unknown>; emit: (event: VolumeEvent) => void }[] = [];
  const channels: Record<string, Call[]> = {
    pane_open: opens as Call[],
    archive_open: opens as Call[],
    find_open: opens as Call[],
    pane_folder_size: sizes as Call[],
    volumes_list: volumes as Call[],
  };
  const invoke = vi.fn((name: CommandName, args: Record<string, unknown> & { onEvent: (event: never) => void }) => {
    channels[name]?.push({ args, emit: args.onEvent });
    return Promise.resolve(answers[name]?.(args as never) ?? null);
  });
  const ipc = { invoke } as unknown as Transport;

  return { ipc, invoke, opens, sizes, volumes, last: () => opens[opens.length - 1] };
}

type Start = Extract<ListEvent, { kind: 'start' }>;

/** What a `start` says about the folder shown besides where it is: the `parent` (by default the folder above `path`) and the path bar. */
type Shown = { parent?: string | null; crumbs?: Crumb[] };

/** The path bar from the root down to the real folder `path`. */
export function crumbsTo(path: string): Crumb[] {
  const names = path.split('/').filter((name) => name !== '');
  return [{ name: '/', path: '/' }, ...names.map((name, index) => ({ name, path: `/${names.slice(0, index + 1).join('/')}` }))];
}

function startOf(path: string, total: number, shown: Shown, place: Pick<Start, 'location' | 'capabilities'>): Start {
  const parent = shown.parent === undefined ? path.slice(0, path.lastIndexOf('/')) || '/' : shown.parent;
  return { kind: 'start', path, parent: path === '/' ? null : parent, crumbs: shown.crumbs ?? [], total, ...place };
}

/** The `start` of the real folder `path`, listing `total` entries. */
export function folderStart(path: string, total: number, shown: Shown = {}): Start {
  return startOf(path, total, shown, { location: { kind: 'folder', path }, capabilities: { writable: true } });
}

/** The `start` of the folder `spot` of an archive, which `path` holds. */
export function archiveStart(path: string, total: number, spot: ArchiveSpot, shown: Shown = {}): Start {
  return startOf(path, total, shown, { location: { kind: 'archive', ...spot }, capabilities: { writable: false } });
}

/** The `start` of the results of the search `id`, which looked in `path`. */
export function resultsStart(path: string, total: number, id: FindId, shown: Shown = {}): Start {
  return startOf(path, total, shown, { location: { kind: 'results', id }, capabilities: { writable: true } });
}

/** Delivers a whole folder on a channel: `start`, one batch with `entries`, `done`. */
export function arrive(
  channel: ReturnType<typeof fakeBackend>['opens'][number],
  path: string,
  entries: Entry[] = [],
  free: number | null = null,
) {
  channel.emit(
    folderStart(path, entries.length, {
      parent: '/',
      crumbs: [
        { name: '/', path: '/' },
        { name: path.split('/').pop() ?? '', path },
      ],
    }),
  );
  channel.emit({ kind: 'batch', offset: 0, entries });
  channel.emit({
    kind: 'done',
    dirs: entries.filter((e) => e.kind === 'dir').length,
    files: entries.filter((e) => e.kind !== 'dir').length,
    bytes: entries.reduce((sum, e) => sum + e.size, 0),
    free,
  });
}

/** Calls `open`, then delivers the folder on the channel it opened. */
export async function deliver(
  backend: ReturnType<typeof fakeBackend>,
  open: () => Promise<void>,
  path: string,
  entries: Entry[],
  free: number | null = null,
) {
  await open();
  const channel = backend.last();
  arrive(channel, path, entries, free);
  return channel;
}
