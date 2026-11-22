import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import type { SizeEvent } from '$lib/ipc/bindings/SizeEvent';
import { IpcFailure } from '$lib/ipc/transport';
import { FolderSizes } from '$lib/panes/sizes.svelte';

import { opsBackend, properties } from '../../../tests/ops';
import { entry } from '../../../tests/panes';
import PropertiesDialog from './PropertiesDialog.svelte';

function open(item = entry('a.pdf'), answers: Parameters<typeof opsBackend>[0] = {}) {
  const backend = opsBackend({ ops_properties: () => properties(), ...answers });
  const sizes = new FolderSizes(backend.ipc, 'left', () => '/work');
  const onClose = vi.fn();
  render(PropertiesDialog, { ipc: backend.ipc, path: '/work', entry: item, sizes, onClose });
  return { ...backend, sizes, onClose };
}

/** The text of a line of the dialog: the value that goes with a label. */
const line = (label: string) => screen.getByText(label, { selector: 'dt' }).nextElementSibling?.textContent;

describe('PropertiesDialog', () => {
  it('reads the item and shows its lines', async () => {
    const { calls } = open();

    expect(await screen.findByText('Permissions')).toBeInTheDocument();
    expect(calls('ops_properties')).toEqual([{ path: '/work', name: 'a.pdf' }]);
    expect(screen.getByRole('dialog', { name: 'Properties of “a.pdf”' })).toBeInTheDocument();
    expect(line('Name')).toBe('a.pdf');
    expect(line('Type')).toBe('File');
    expect(line('Location')).toBe('/work');
    expect(line('Size')).toBe('1.5 KB (1,536 bytes)');
    expect(line('Modified')).toBe('2026-11-20 07:58:12');
    expect(line('Created')).toBe('2026-10-01 08:30:00');
    expect(line('Accessed')).toBe('Unknown');
    expect(line('Permissions')).toBe('0644 rw-r--r--');
    expect(line('Owner')).toBe('ann');
    expect(line('Group')).toBe('staff');
    expect(screen.queryByText('Attributes')).not.toBeInTheDocument();
    expect(screen.queryByText('Points to')).not.toBeInTheDocument();
  });

  it('shows names and paths that reorder text with their characters written out', async () => {
    const { calls } = open(entry('invoice\u202Efdp.exe'), {
      ops_properties: () => properties({ name: 'invoice\u202Efdp.exe', linkTarget: '/t/a\u202Eb', owner: 'o\u200Bw' }),
    });

    expect(await screen.findByText('Permissions')).toBeInTheDocument();
    expect(calls('ops_properties')).toEqual([{ path: '/work', name: 'invoice\u202Efdp.exe' }]);
    expect(screen.getByRole('dialog', { name: 'Properties of “invoice⟨U+202E⟩fdp.exe”' })).toBeInTheDocument();
    expect(line('Name')).toBe('invoice⟨U+202E⟩fdp.exe');
    expect(line('Points to')).toBe('/t/a⟨U+202E⟩b');
    expect(line('Owner')).toBe('o⟨U+200B⟩w');
  });

  it('says it is reading until the answer comes, and has Close focused', async () => {
    open(undefined, { ops_properties: () => new Promise(() => {}) });

    expect(screen.getByText('Reading…')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Close' })).toHaveFocus();
  });

  it('Close and Escape leave it', async () => {
    const { onClose } = open();
    await screen.findByText('Permissions');

    await fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('a refusal is shown', async () => {
    open(undefined, {
      ops_properties: async () => {
        throw new IpcFailure({ code: 'forbidden', message: 'You do not have permission to read this item’s properties.' });
      },
    });

    expect(await screen.findByRole('alert')).toHaveTextContent('You do not have permission to read');
    expect(screen.queryByText('Reading…')).not.toBeInTheDocument();
  });

  it('a link shows where it leads', async () => {
    open(entry('current', 'dir'), {
      ops_properties: () => properties({ name: 'current', kind: 'dir', size: 0, linkTarget: '../releases/v2' }),
    });

    expect(await screen.findByText('Points to')).toBeInTheDocument();
    expect(line('Type')).toBe('Symbolic link to a folder');
    expect(line('Points to')).toBe('../releases/v2');
  });

  it('a folder is measured, as the pane does, and shows the size once it is known', async () => {
    const { calls, sizes } = open(entry('docs', 'dir'), {
      ops_properties: () => properties({ name: 'docs', kind: 'dir', size: 0 }),
      pane_folder_size: () => null,
    });

    expect(await screen.findByText('Permissions')).toBeInTheDocument();
    expect(calls('pane_folder_size')).toMatchObject([{ pane: 'left', path: '/work', name: 'docs' }]);
    expect(line('Size')).toBe('Calculating…');

    const [{ onEvent }] = calls('pane_folder_size') as unknown as { onEvent: (event: SizeEvent) => void }[];
    onEvent({ kind: 'done', bytes: 5 * 1024 * 1024 });

    await vi.waitFor(() => expect(line('Size')).toBe('5.0 MB (5,242,880 bytes)'));
    expect(sizes.sizeOf('docs')).toBe(5 * 1024 * 1024);
  });

  it('shows the Windows attributes, and no permissions or owner', async () => {
    open(undefined, {
      ops_properties: () =>
        properties({
          mode: null,
          owner: null,
          group: null,
          windows: { readOnly: true, hidden: false, system: false, archive: true },
        }),
    });

    expect(await screen.findByText('Attributes')).toBeInTheDocument();
    expect(line('Attributes')).toBe('Read-only, Archive');
    expect(screen.queryByText('Permissions')).not.toBeInTheDocument();
    expect(screen.queryByText('Owner')).not.toBeInTheDocument();
  });

  it('says None for a file with no attribute set', async () => {
    open(undefined, {
      ops_properties: () =>
        properties({ mode: null, windows: { readOnly: false, hidden: false, system: false, archive: false } }),
    });

    expect(await screen.findByText('Attributes')).toBeInTheDocument();
    expect(line('Attributes')).toBe('None');
  });
});
