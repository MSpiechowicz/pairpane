// Test helpers for the whole app: a backend with a left pane in `/left` and a right pane in `/right`.
import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { expect, type Mock, vi } from 'vitest';

import type { ArchiveSpot } from '$lib/ipc/bindings/ArchiveSpot';
import type { Entry } from '$lib/ipc/bindings/Entry';
import type { ListEvent } from '$lib/ipc/bindings/ListEvent';
import type { OpsEvent } from '$lib/ipc/bindings/OpsEvent';

import App from '../src/App.svelte';
import { entry } from './panes';

export type Answers = Record<string, (args: never) => unknown>;

/** The test sends the listings of the panes and the job events; `invoke` is the mocked transport's. */
export function appWorld(invoke: Mock, answers: Answers = {}) {
  const opens: { pane: string; emit: (event: ListEvent) => void }[] = [];
  let ops: (event: OpsEvent) => void = () => {};
  invoke.mockImplementation(async (name: string, args: { pane?: string; onEvent?: never }) => {
    if (name === 'pane_open' || name === 'archive_open') {
      opens.push({ pane: args.pane as string, emit: args.onEvent as never });
    } else if (name === 'ops_subscribe') {
      ops = args.onEvent as never;
    }

    return answers[name]?.(args as never) ?? null;
  });

  /** Lists a folder in a pane (the one whose listing was asked last). */
  const list = (side: 'left' | 'right', path: string, entries: Entry[]) => {
    const channel = opens.filter((open) => open.pane === side).at(-1)!;
    channel.emit({
      kind: 'start',
      path,
      parent: '/',
      crumbs: [
        { name: '/', path: '/' },
        { name: path.slice(1), path },
      ],
      total: entries.length,
    });
    channel.emit({ kind: 'batch', offset: 0, entries });
    channel.emit({ kind: 'done', dirs: 0, files: entries.length, bytes: 0, free: null });
  };

  /** Lists a folder of an archive in `/<side>` in a pane (the one whose listing was asked last). */
  const listIn = (side: 'left' | 'right', spot: ArchiveSpot, entries: Entry[]) => {
    const channel = opens.filter((open) => open.pane === side).at(-1)!;
    channel.emit({
      kind: 'start',
      path: `/${side}`,
      parent: '/',
      crumbs: [
        { name: '/', path: '/' },
        { name: side, path: `/${side}` },
        { name: spot.archive.split('/').pop() ?? '', path: spot.archive, inside: '' },
      ],
      total: entries.length,
      archive: spot,
    });
    channel.emit({ kind: 'batch', offset: 0, entries });
    channel.emit({ kind: 'done', dirs: 0, files: entries.length, bytes: 0, free: null });
  };

  const sent = (name: string) => invoke.mock.calls.filter(([command]) => command === name).map(([, args]) => args);

  return {
    sent,
    list,
    listIn,
    ops: (...events: OpsEvent[]) => events.forEach((event) => ops(event)),
    ready: async () => {
      await vi.waitFor(() => expect(opens.length).toBeGreaterThanOrEqual(2));
      await vi.waitFor(() => expect(sent('ops_subscribe')).toHaveLength(1));
    },
  };
}

export const pane = (name: string) => screen.getByRole('region', { name });
export const listbox = (side: 'Left' | 'Right') => screen.getByRole('listbox', { name: `${side} pane files` });
export const key = (target: Element | Window, k: string, init: KeyboardEventInit = {}) =>
  fireEvent.keyDown(target, { key: k, ...init });

/** Both panes listed (`a.pdf`, `b.txt`, `sub` in the left one, `c.md` in the right one); the left cursor on `a.pdf`. */
export async function appShown(invoke: Mock, answers: Answers = {}) {
  const backend = appWorld(invoke, answers);
  render(App);
  await backend.ready();
  backend.list('left', '/left', [entry('a.pdf'), entry('b.txt', 'file'), entry('sub', 'dir')]);
  backend.list('right', '/right', [entry('c.md')]);
  await tick();
  await key(listbox('Left'), 'ArrowDown');
  return backend;
}
