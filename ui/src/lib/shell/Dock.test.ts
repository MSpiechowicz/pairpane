import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import type { ShellEvent } from '$lib/ipc/bindings/ShellEvent';
import type { Transport } from '$lib/ipc/transport';

import { CommandLine } from './command-line.svelte';
import Dock from './Dock.svelte';

describe('Dock', () => {
  it('a click on a function key reports the action it stands for', async () => {
    const onAction = vi.fn();
    render(Dock, { onAction });

    await fireEvent.click(screen.getByRole('button', { name: 'Copy (F5)' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Move (F6)' }));

    expect(onAction.mock.calls).toEqual([['copy'], ['move']]);
  });

  it('F7 and F8 report too', async () => {
    const onAction = vi.fn();
    render(Dock, { onAction });

    await fireEvent.click(screen.getByRole('button', { name: 'New folder (F7)' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Delete (F8)' }));

    expect(onAction.mock.calls).toEqual([['newFolder'], ['trash']]);
  });

  it('the keys can be clicked with nobody listening', async () => {
    render(Dock);

    await fireEvent.click(screen.getByRole('button', { name: 'Delete (F8)' }));
  });

  it('F5 is called Extract inside an archive and Copy elsewhere, and still reports copy', async () => {
    const onAction = vi.fn();
    const { rerender } = render(Dock, { onAction });
    expect(screen.getByRole('button', { name: 'Copy (F5)' })).toBeInTheDocument();

    await rerender({ onAction, inArchive: true });

    expect(screen.queryByRole('button', { name: 'Copy (F5)' })).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Extract (F5)' }));
    expect(onAction.mock.calls).toEqual([['copy']]);
    expect(screen.getByRole('button', { name: 'Move (F6)' })).toBeInTheDocument();
  });

  it('over search results Move, Edit and New folder are marked as unavailable, with why; a click still reports, so the card can say it', async () => {
    const onAction = vi.fn();
    render(Dock, { onAction, inResults: true });

    const move = screen.getByRole('button', { name: 'Move (F6)' });
    const folder = screen.getByRole('button', { name: 'New folder (F7)' });
    const edit = screen.getByRole('button', { name: 'Edit (F4)' });
    for (const key of [move, folder, edit]) {
      expect(key).toHaveAttribute('aria-disabled', 'true');
    }

    expect(edit).toHaveAttribute('title', 'Open the file’s folder first.');
    expect(move).toHaveAttribute('title', 'Open the file’s folder first.');
    for (const key of ['Copy (F5)', 'Delete (F8)', 'View (F3)']) {
      expect(screen.getByRole('button', { name: key })).not.toHaveAttribute('aria-disabled');
    }

    await fireEvent.click(move);
    expect(onAction.mock.calls).toEqual([['move']]);
  });

  it('in a folder no key is marked as unavailable', () => {
    render(Dock);

    expect(document.querySelectorAll('[aria-disabled]')).toHaveLength(0);
  });
});

describe('the command line of the dock', () => {
  function withLine() {
    const sinks: ((event: ShellEvent) => void)[] = [];
    const invoke = vi.fn((name: string, args: Record<string, unknown>) => {
      if (name === 'shell_run') {
        sinks.push(args.onEvent as (event: ShellEvent) => void);
      }
      return Promise.resolve(1);
    });
    const commandLine = new CommandLine({ invoke } as unknown as Transport, () => ({ folder: '/home/ann' }), vi.fn());
    render(Dock, { commandLine });
    return { invoke, sinks, input: screen.getByRole('textbox', { name: 'Shell command' }) };
  }

  it('runs the line on Enter, shows the output above the dock, and stops and closes it', async () => {
    const { invoke, sinks, input } = withLine();

    await fireEvent.input(input, { target: { value: 'ls' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    await vi.waitFor(() => expect(input).toHaveValue(''));

    sinks[0]({ kind: 'output', text: 'notes.txt\nplan.md\n' });
    const output = await screen.findByRole('region', { name: 'Command output' });
    // Lines stay lines (not written out as ⟨U+000A⟩).
    expect(output.querySelector('pre')?.textContent).toBe('notes.txt\nplan.md\n');
    expect(output).toHaveTextContent('Running…');

    await fireEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(invoke).toHaveBeenCalledWith('shell_stop', { id: 1 });
    sinks[0]({ kind: 'done', code: null, stopped: true, truncated: false });
    await vi.waitFor(() => expect(output).toHaveTextContent('Stopped'));

    await fireEvent.keyDown(input, { key: 'Escape' });
    expect(screen.queryByRole('region', { name: 'Command output' })).not.toBeInTheDocument();
  });

  it('brings back the lines run with the arrow keys', async () => {
    const { sinks, input } = withLine();
    await fireEvent.input(input, { target: { value: 'git status' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    await vi.waitFor(() => expect(sinks).toHaveLength(1));
    sinks[0]({ kind: 'done', code: 0, stopped: false, truncated: false });

    await fireEvent.keyDown(input, { key: 'ArrowUp' });
    expect(input).toHaveValue('git status');
    await fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(input).toHaveValue('');
  });
});
