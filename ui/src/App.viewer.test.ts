import { fireEvent, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));
// A PDF never finishes opening here: pdf.js itself is not what these tests are about.
vi.mock('$lib/viewer/pdf', () => ({ openPdf: () => new Promise(() => {}) }));

import type { ViewKind } from '$lib/ipc/bindings/ViewKind';

import { type Answers, appShown, key, listbox, pane } from '../tests/app';
import { FakeFile, numbered, viewAnswers } from '../tests/view';

const shown = (answers: Answers = {}, kind?: (name: string) => ViewKind) =>
  appShown(
    invoke,
    viewAnswers(
      new FakeFile(numbered(10)),
      kind ?? ((name) => (name === 'a.pdf' ? { kind: 'pdf' } : { kind: 'text' })),
      answers,
    ),
  );

const overlay = () => screen.queryByRole('dialog');
const quick = () => screen.queryByRole('region', { name: 'Quick view' });

beforeEach(() => {
  invoke.mockReset();
  window.localStorage.clear();
  vi.useRealTimers();
});

describe('F3 viewer', () => {
  it('F3 opens the file under the cursor in a full-window viewer', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'F3');

    expect(await screen.findByRole('dialog', { name: 'a.pdf' })).toBeInTheDocument();
    expect(backend.sent('view_open')).toEqual([{ path: '/left', name: 'a.pdf' }]);
  });

  it('Escape closes it, ends the session and gives the list the focus back', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'F3');
    await screen.findByRole('dialog', { name: 'a.pdf' });
    await vi.waitFor(() => expect(backend.sent('view_open')).toHaveLength(1));

    await key(window, 'Escape');

    expect(overlay()).not.toBeInTheDocument();
    await vi.waitFor(() => expect(backend.sent('view_close')).toEqual([{ session: 1 }]));
    await vi.waitFor(() => expect(listbox('Left')).toHaveFocus());
  });

  it('does nothing on a folder, on the parent row or when there is no file', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'ArrowUp'); // `..`
    await key(listbox('Left'), 'F3');
    await key(listbox('Left'), 'ArrowDown');
    await key(listbox('Left'), 'ArrowDown');
    await key(listbox('Left'), 'ArrowDown'); // `sub`
    await key(listbox('Left'), 'F3');

    expect(overlay()).not.toBeInTheDocument();
    expect(backend.sent('view_open')).toHaveLength(0);
  });

  it('the dock`s F3 View button does the same as the key', async () => {
    const backend = await shown();

    await fireEvent.click(screen.getByRole('button', { name: 'View (F3)' }));

    expect(await screen.findByRole('dialog', { name: 'a.pdf' })).toBeInTheDocument();
    expect(backend.sent('view_open')).toEqual([{ path: '/left', name: 'a.pdf' }]);
  });

  it('the pane shortcuts are off while it is open: F5 does not copy', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'F3');
    await screen.findByRole('dialog', { name: 'a.pdf' });

    await key(window, 'F5');
    await key(window, 'F7');
    await key(window, 't', { ctrlKey: true });

    expect(screen.getAllByRole('dialog')).toHaveLength(1);
    expect(screen.queryByRole('dialog', { name: /Copy/ })).not.toBeInTheDocument();
    expect(backend.sent('ops_start')).toHaveLength(0);
  });

  it('F3 on an image shows the image, and closing the viewer lets go of it', async () => {
    const made = vi.fn(() => 'blob:picture');
    const revoked = vi.fn();
    const original = { create: URL.createObjectURL, revoke: URL.revokeObjectURL };
    URL.createObjectURL = made;
    URL.revokeObjectURL = revoked;

    try {
      await shown({}, () => ({ kind: 'image', mime: 'image/png' }));

      await key(listbox('Left'), 'F3');

      expect(await screen.findByRole('img', { name: 'a.pdf' })).toHaveAttribute('src', 'blob:picture');
      await key(window, 'Escape');
      expect(revoked).toHaveBeenCalledWith('blob:picture');
    } finally {
      URL.createObjectURL = original.create;
      URL.revokeObjectURL = original.revoke;
    }
  });

  it('shows why a file cannot be viewed: a link is refused with the backend`s message', async () => {
    await shown({
      view_open: () => Promise.reject({ code: 'invalidInput', message: 'The viewer opens regular files, not links' }),
    });

    await key(listbox('Left'), 'F3');

    expect(await screen.findByRole('alert')).toHaveTextContent('The viewer opens regular files, not links');
    await key(window, 'Escape');
    expect(overlay()).not.toBeInTheDocument();
  });
});

describe('Quick view', () => {
  it('Ctrl+Q swaps the other pane for a quick view of the file under the cursor', async () => {
    const backend = await shown();

    await key(listbox('Left'), 'q', { ctrlKey: true });

    expect(quick()).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Right pane' })).not.toBeInTheDocument();
    expect(pane('Left pane')).toBeInTheDocument();
    await vi.waitFor(() => expect(backend.sent('view_open')).toEqual([{ path: '/left', name: 'a.pdf' }]));
    expect(await screen.findByText('Loading…')).toBeInTheDocument();
  });

  it('follows the cursor once it rests, closing the session of the file it left', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'q', { ctrlKey: true });
    await vi.waitFor(() => expect(backend.sent('view_open')).toHaveLength(1));

    vi.useFakeTimers();
    await key(listbox('Left'), 'ArrowDown');
    await vi.advanceTimersByTimeAsync(100);
    expect(backend.sent('view_open')).toHaveLength(1);
    expect(backend.sent('view_close')).toHaveLength(0);

    await vi.advanceTimersByTimeAsync(60);
    expect(backend.sent('view_open')).toEqual([
      { path: '/left', name: 'a.pdf' },
      { path: '/left', name: 'b.txt' },
    ]);
    expect(backend.sent('view_close')).toEqual([{ session: 1 }]);
    vi.useRealTimers();
    expect(await screen.findByText('line 0')).toBeInTheDocument();
  });

  it('opens nothing for the files the cursor only passes', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'q', { ctrlKey: true });
    await vi.waitFor(() => expect(backend.sent('view_open')).toHaveLength(1));

    vi.useFakeTimers();
    await key(listbox('Left'), 'ArrowDown');
    await vi.advanceTimersByTimeAsync(100);
    await key(listbox('Left'), 'ArrowUp');
    await vi.advanceTimersByTimeAsync(400);

    expect(backend.sent('view_open')).toHaveLength(1);
    expect(backend.sent('view_close')).toHaveLength(0);
  });

  it('a folder under the cursor is a placeholder with no backend call', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'ArrowDown');
    await key(listbox('Left'), 'ArrowDown');

    await key(listbox('Left'), 'q', { ctrlKey: true });

    expect(await screen.findByText('Folder')).toBeInTheDocument();
    expect(backend.sent('view_open')).toHaveLength(0);
  });

  it('Tab does not switch panes while it is open', async () => {
    await shown();
    await key(listbox('Left'), 'q', { ctrlKey: true });

    const proceeded = await key(listbox('Left'), 'Tab');

    expect(proceeded).toBe(true); // not claimed: the key was left alone
    expect(pane('Left pane')).toHaveAttribute('aria-current', 'true');
  });

  it('Ctrl+Q again brings the pane back and ends the quick view`s session', async () => {
    const backend = await shown();
    await key(listbox('Left'), 'q', { ctrlKey: true });
    await vi.waitFor(() => expect(backend.sent('view_open')).toHaveLength(1));
    await screen.findByText('Loading…');

    await key(listbox('Left'), 'q', { ctrlKey: true });

    expect(quick()).not.toBeInTheDocument();
    expect(pane('Right pane')).toBeInTheDocument();
    await vi.waitFor(() => expect(backend.sent('view_close')).toEqual([{ session: 1 }]));
    await key(listbox('Left'), 'Tab');
    expect(pane('Right pane')).toHaveAttribute('aria-current', 'true');
  });
});
