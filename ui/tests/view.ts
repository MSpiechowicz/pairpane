// Test helpers for the viewer: an in-memory file that answers the `view_*` commands the way the backend does.
import { expect, vi } from 'vitest';

import type { SearchHit } from '$lib/ipc/bindings/SearchHit';
import type { SearchQuery } from '$lib/ipc/bindings/SearchQuery';
import type { TextQuery } from '$lib/ipc/bindings/TextQuery';
import type { TextWindow } from '$lib/ipc/bindings/TextWindow';
import type { ViewKind } from '$lib/ipc/bindings/ViewKind';
import type { ViewOpened } from '$lib/ipc/bindings/ViewOpened';
import type { CommandName } from '$lib/ipc/commands';
import { ViewSession } from '$lib/viewer/session.svelte';

import type { Answers } from './app';
import { fakeBackend } from './panes';

const encoder = new TextEncoder();

/** A file of bytes. Lines end at `\n`; a last line without one still counts. */
export class FakeFile {
  readonly bytes: Uint8Array;
  /** The offset each line starts at. */
  readonly starts: number[] = [];

  constructor(content: string | Uint8Array) {
    this.bytes = typeof content === 'string' ? encoder.encode(content) : content;
    let at = 0;
    while (at < this.bytes.length) {
      this.starts.push(at);
      const newline = this.bytes.indexOf(10, at);
      at = newline < 0 ? this.bytes.length : newline + 1;
    }
  }

  get size() {
    return this.bytes.length;
  }

  #endOf(line: number): number {
    return this.starts[line + 1] ?? this.size;
  }

  #lines(from: number, to: number, encoding: string) {
    const decoder = new TextDecoder(encoding);

    return this.starts.slice(from, to).map((offset, index) => {
      const end = this.#endOf(from + index);
      const text = decoder.decode(this.bytes.subarray(offset, this.bytes[end - 1] === 10 ? end - 1 : end));
      return { offset, text, continues: false };
    });
  }

  /** What `view_text` answers: forward lines from the offset (its line's start when snapped), or the ones before it. */
  window(query: TextQuery): TextWindow {
    const before = this.starts.filter((start) => start < query.offset).length;
    let first = before;
    if (!query.backward) {
      const inside = this.starts.filter((start) => start <= query.offset).length - 1;
      first = query.snap ? Math.max(0, inside) : this.starts.findIndex((start) => start >= query.offset);
      first = first < 0 ? this.starts.length : first;
    }

    const [from, to] = query.backward ? [Math.max(0, before - query.lines), before] : [first, first + query.lines];
    const lines = this.#lines(from, to, query.encoding);
    const end = lines.length === 0 ? query.offset : this.#endOf(Math.min(to, this.starts.length) - 1);

    return {
      start: lines[0]?.offset ?? query.offset,
      end,
      lines,
      atStart: (lines[0]?.offset ?? query.offset) === 0,
      atEnd: end >= this.size,
    };
  }

  /** What `view_search` answers: the first match at or after `from`, or the last one before it. */
  search(query: SearchQuery): SearchHit | null {
    const needle =
      query.needle.kind === 'hex' ? Uint8Array.from(query.needle.bytes) : encoder.encode(query.needle.text);
    const fold = query.needle.kind === 'text' && !query.needle.caseSensitive;
    const same = (a: number, b: number) => a === b || (fold && String.fromCharCode(a).toLowerCase() === String.fromCharCode(b).toLowerCase());
    const matches = (at: number) => needle.every((byte, index) => same(this.bytes[at + index], byte));

    const last = this.size - needle.length;
    const candidates = query.backward
      ? Array.from({ length: Math.min(query.from - 1, last) + 1 }, (_, at) => Math.min(query.from - 1, last) - at)
      : Array.from({ length: Math.max(0, last - query.from + 1) }, (_, at) => query.from + at);
    const found = candidates.find(matches);

    return found === undefined ? null : { offset: found, len: needle.length };
  }
}

/** `count` lines `line 0`, `line 1`… */
export const numbered = (count: number) => Array.from({ length: count }, (_, index) => `line ${index}\n`).join('');

/**
 * The answers of a fake backend for the viewer: each `view_open` is a new session (1, 2, …) on `file`, of the kind
 * `kind` says for that name. `view_text`, `view_bytes` and `view_search` read the file.
 */
export function viewAnswers(
  file: FakeFile,
  kind: (name: string) => ViewKind = () => ({ kind: 'text' }),
  extra: Answers = {},
): Answers {
  let sessions = 0;
  const open = (args: { name: string }): ViewOpened => {
    sessions += 1;
    return { session: sessions, name: args.name, size: file.size, modified: null, kind: kind(args.name), encoding: 'UTF-8' };
  };

  return {
    view_open: open,
    // An entry of an archive is opened the same way: the backend answers with the session of its extracted copy.
    archive_view_open: open,
    view_text: (args: { query: TextQuery }) => file.window(args.query),
    view_bytes: (args: { offset: number; length: number }) =>
      file.bytes.slice(args.offset, args.offset + args.length).buffer,
    view_search: (args: { query: SearchQuery }) => file.search(args.query),
    ...extra,
  };
}

/** A fake backend serving `file`, and a session opened on it (waited for). */
export async function openedSession(file: FakeFile, kind?: (name: string) => ViewKind, extra: Answers = {}) {
  const backend = fakeBackend(viewAnswers(file, kind, extra) as Partial<Record<CommandName, (args: never) => unknown>>);
  const session = new ViewSession(backend.ipc, '/dir', 'file.txt');
  await vi.waitFor(() => expect(session.status).toBe('ready'));

  /** The arguments of every call of a command. */
  const sent = (name: string) => backend.invoke.mock.calls.filter(([command]) => command === name).map(([, args]) => args);

  return { backend, session, sent };
}
