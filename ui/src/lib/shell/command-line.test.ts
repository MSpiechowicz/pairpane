import { describe, expect, it, vi } from 'vitest';

import type { ShellEvent } from '$lib/ipc/bindings/ShellEvent';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { CommandLine, MOST_REMEMBERED, type Place, placeOf } from './command-line.svelte';

type Sink = (event: ShellEvent) => void;

function setup(place: Place = { folder: '/home/ann' }, answer: (name: string) => Promise<unknown> = () => Promise.resolve(7)) {
  const sinks: Sink[] = [];
  const invoke = vi.fn((name: string, args: Record<string, unknown>) => {
    if (name === 'shell_run') {
      sinks.push(args.onEvent as Sink);
    }
    return answer(name);
  });
  const say = vi.fn();
  const line = new CommandLine({ invoke } as unknown as Transport, () => place, say);
  const sent = (name: string) => invoke.mock.calls.filter(([command]) => command === name).map(([, args]) => args);

  return { line, say, sinks, sent };
}

describe('CommandLine', () => {
  it('runs a line in the active folder and shows its output and how it ended', async () => {
    const s = setup();

    expect(await s.line.run('  ls -la  ')).toBe(true);

    expect(s.sent('shell_run')).toEqual([expect.objectContaining({ path: '/home/ann', command: 'ls -la' })]);
    expect(s.line.shown).toMatchObject({ line: 'ls -la', status: 'running' });
    s.sinks[0]({ kind: 'output', text: 'a\n' });
    s.sinks[0]({ kind: 'output', text: 'b\n' });
    s.sinks[0]({ kind: 'done', code: 2, stopped: false, truncated: true });

    expect(s.line.shown).toMatchObject({ output: 'a\nb\n', status: 'done', code: 2, note: expect.stringMatching(/too long/) });
    expect(s.line.running).toBe(false);
  });

  it('keeps the id of a command whose output came before the id did, so it can be stopped', async () => {
    let answer!: (id: number) => void;
    const s = setup(undefined, (name) => (name === 'shell_run' ? new Promise((resolve) => (answer = resolve)) : Promise.resolve(null)));

    const running = s.line.run('make');
    await vi.waitFor(() => expect(s.sinks).toHaveLength(1));
    s.sinks[0]({ kind: 'output', text: 'building\n' });
    answer(3);
    await running;

    s.line.stop();
    expect(s.sent('shell_stop')).toEqual([{ id: 3 }]);
  });

  it('runs nothing outside a folder on disk, nor while a command runs, and says why', async () => {
    const away = setup({ refusal: 'Commands run in a folder on disk: open one first.' });
    expect(await away.line.run('ls')).toBe(false);
    expect(away.say).toHaveBeenCalledWith('Commands run in a folder on disk: open one first.', true);

    const busy = setup();
    await busy.line.run('sleep 9');
    expect(await busy.line.run('ls')).toBe(false);
    expect(busy.say).toHaveBeenCalledWith('A command is running: stop it first.', true);
    expect(busy.sent('shell_run')).toHaveLength(1);
  });

  it('shows why a command was not run', async () => {
    const s = setup(undefined, () => Promise.reject(new IpcFailure({ code: 'invalidInput', message: 'the command line was not allowed' })));

    await s.line.run('ls');

    expect(s.line.shown).toMatchObject({ status: 'failed', note: 'the command line was not allowed' });
  });

  it('a close stops the command and leaves out what it still sends', async () => {
    const s = setup();
    await s.line.run('sleep 9');

    s.line.close();
    s.sinks[0]({ kind: 'output', text: 'late' });

    expect(s.sent('shell_stop')).toEqual([{ id: 7 }]);
    expect(s.line.shown).toBeUndefined();
  });

  it('remembers the lines for the arrow keys, each once, the newest last', async () => {
    const s = setup();
    for (const text of ['one', 'two', 'one']) {
      await s.line.run(text);
      s.sinks.at(-1)!({ kind: 'done', code: 0, stopped: false, truncated: false });
    }

    expect([s.line.recall(-1), s.line.recall(-1), s.line.recall(-1)]).toEqual(['one', 'two', undefined]);
    expect([s.line.recall(1), s.line.recall(1), s.line.recall(1)]).toEqual(['one', '', undefined]);
    expect(MOST_REMEMBERED).toBe(50);
  });

  it('opens the terminal in the active folder and says when it cannot', async () => {
    const s = setup(undefined, () => Promise.reject(new IpcFailure({ code: 'process', message: 'no terminal was found' })));

    await s.line.terminal();

    expect(s.sent('terminal_open')).toEqual([{ path: '/home/ann' }]);
    expect(s.say).toHaveBeenCalledWith('Cannot open a terminal: no terminal was found', true);
  });

  it('a pane in an archive, on search results or not listed yet has no folder for a command', () => {
    expect(placeOf({ folder: '/a', readonly: true, results: undefined })).toHaveProperty('refusal');
    expect(placeOf({ folder: '/a', readonly: false, results: 4 })).toHaveProperty('refusal');
    expect(placeOf({ folder: undefined, readonly: false, results: undefined })).toEqual({
      refusal: 'Wait until the folder has been listed.',
    });
    expect(placeOf({ folder: '/a', readonly: false, results: undefined })).toEqual({ folder: '/a' });
  });
});
