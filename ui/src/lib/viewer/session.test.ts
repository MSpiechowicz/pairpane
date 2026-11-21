import { describe, expect, it, vi } from 'vitest';

import type { ViewKind } from '$lib/ipc/bindings/ViewKind';
import type { ViewOpened } from '$lib/ipc/bindings/ViewOpened';
import type { Transport } from '$lib/ipc/transport';

import { ENCODINGS, encodingChoices } from './encodings';
import { ViewSession } from './session.svelte';

const opened = (kind: ViewKind, encoding = 'UTF-8'): ViewOpened => ({
  session: 7,
  name: 'f',
  size: 10,
  modified: null,
  kind,
  encoding,
});

/** A transport whose `view_open` answers as `open` says, and which records every call. */
function transport(open: () => Promise<ViewOpened>) {
  const invoke = vi.fn((name: string) => (name === 'view_open' ? open() : Promise.resolve(null)));

  return { ipc: { invoke } as unknown as Transport, invoke };
}

async function session(kind: ViewKind, encoding?: string) {
  const world = transport(() => Promise.resolve(opened(kind, encoding)));
  const made = new ViewSession(world.ipc, '/dir', 'f');
  await vi.waitFor(() => expect(made.status).toBe('ready'));

  return { ...world, made };
}

describe('ViewSession', () => {
  it('opens the file by its folder and name and takes the encoding that was detected', async () => {
    const { made, invoke } = await session({ kind: 'text' }, 'windows-1250');

    expect(invoke).toHaveBeenCalledWith('view_open', { path: '/dir', name: 'f' });
    expect(made.id).toBe(7);
    expect(made.encoding).toBe('windows-1250');
  });

  it('reads what is not text as UTF-8 until told otherwise', async () => {
    expect((await session({ kind: 'binary' }, '')).made.encoding).toBe('UTF-8');
  });

  it('a text file is shown as text, a binary one as bytes; both can be seen as the other', async () => {
    const text = (await session({ kind: 'text' })).made;
    const binary = (await session({ kind: 'binary' })).made;

    expect([text.shown, text.modes]).toEqual(['text', ['text', 'hex']]);
    expect([binary.shown, binary.modes]).toEqual(['hex', ['text', 'hex']]);

    text.mode = 'hex';
    binary.mode = 'text';
    expect([text.shown, binary.shown]).toEqual(['hex', 'text']);
  });

  it.each([
    { kind: 'markdown' },
    { kind: 'image', mime: 'image/png' },
    { kind: 'pdf' },
    { kind: 'audio', mime: 'audio/mpeg' },
    { kind: 'video', mime: 'video/mp4' },
    { kind: 'archive', format: 'zip' },
  ] as const)('a $kind file has a preview, and Text and Hex stay available', async (kind) => {
    const { made } = await session(kind);

    expect(made.shown).toBe('preview');
    expect(made.modes).toEqual(['auto', 'text', 'hex']);
    made.mode = 'text';
    expect(made.shown).toBe('text');
    made.mode = 'hex';
    expect(made.shown).toBe('hex');
  });

  it('only text and rendered Markdown use the encoding', async () => {
    const markdown = (await session({ kind: 'markdown' })).made;
    const pdf = (await session({ kind: 'pdf' })).made;

    expect([markdown.decoded, pdf.decoded]).toEqual([true, false]);
    pdf.mode = 'text';
    expect(pdf.decoded).toBe(true);
    pdf.mode = 'hex';
    expect(pdf.decoded).toBe(false);
  });

  it('closes its backend session once, however often it is closed', async () => {
    const { made, invoke } = await session({ kind: 'text' });

    made.close();
    made.close();

    expect(invoke.mock.calls.filter(([name]) => name === 'view_close')).toEqual([['view_close', { session: 7 }]]);
  });

  it('closed before the backend answered, it lets go of the session when it arrives', async () => {
    let answer: (value: ViewOpened) => void = () => {};
    const { ipc, invoke } = transport(() => new Promise((resolve) => (answer = resolve)));
    const made = new ViewSession(ipc, '/dir', 'f');

    made.close();
    expect(invoke.mock.calls.filter(([name]) => name === 'view_close')).toHaveLength(0);
    answer(opened({ kind: 'text' }));

    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('view_close', { session: 7 }));
    expect(made.status).toBe('opening');
  });

  it('keeps the backend`s words when it refuses the file', async () => {
    const { ipc } = transport(() => Promise.reject({ code: 'invalidInput', message: 'The viewer opens regular files, not links' }));
    const made = new ViewSession(ipc, '/dir', 'link');

    await vi.waitFor(() => expect(made.status).toBe('failed'));

    expect(made.error).toBe('The viewer opens regular files, not links');
    made.close(); // nothing was opened: nothing to close
  });
});

describe('encodingChoices', () => {
  it('offers the list, with a detected encoding that is not in it put first', () => {
    expect(encodingChoices('windows-1250')).toBe(ENCODINGS);
    expect(encodingChoices('')).toBe(ENCODINGS);
    expect(encodingChoices('x-odd')).toEqual(['x-odd', ...ENCODINGS]);
    expect(ENCODINGS).toContain('UTF-16LE');
  });
});
