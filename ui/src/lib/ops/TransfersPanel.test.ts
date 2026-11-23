import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import type { OpsEvent } from '$lib/ipc/bindings/OpsEvent';

import { job, opsBackend } from '../../../tests/ops';
import TransfersChip from './TransfersChip.svelte';
import TransfersPanel from './TransfersPanel.svelte';
import { Transfers } from './transfers.svelte';

async function store() {
  const backend = opsBackend();
  const transfers = new Transfers(backend.ipc);
  await transfers.subscribe();
  return { ...backend, transfers };
}

const progress = (id: number, doneBytes: number, totalBytes: number): OpsEvent => ({
  kind: 'progress',
  id,
  doneBytes,
  totalBytes,
  doneFiles: 0,
  totalFiles: 1,
  current: null,
  bytesPerSec: 10 * 1024 * 1024,
  etaSecs: 65,
});

/** The progress of a job that has gone through `files` files. */
const counted = (id: number, files: number): OpsEvent => ({
  kind: 'progress',
  id,
  doneBytes: 0,
  totalBytes: 0,
  doneFiles: files,
  totalFiles: files,
  current: null,
  bytesPerSec: 0,
  etaSecs: null,
});

const ended = (id: number, extra: Partial<Extract<OpsEvent, { kind: 'ended' }>> = {}): OpsEvent => ({
  kind: 'ended',
  id,
  outcome: { kind: 'done' },
  skipped: 0,
  errors: [],
  moreErrors: 0,
  ...extra,
});

describe('TransfersPanel', () => {
  it('names an extraction, where it goes and the step it is in', async () => {
    const { transfers, send } = await store();
    send({ kind: 'added', job: job(1, { label: 'a.zip', kind: 'extract', phase: 'extracting', to: '/out' }) });
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    expect(screen.getByText('Extract')).toBeInTheDocument();
    expect(screen.getByText('→ /out')).toBeInTheDocument();
    expect(screen.getByText('Extracting…')).toBeInTheDocument();
  });

  it('writes out controls and invisible characters of a job’s name, target and errors', async () => {
    const { transfers, send } = await store();
    const error = (message: string) => ({ code: 'io' as const, message });
    send(
      { kind: 'added', job: job(1, { label: 'a\u202Efdp.exe', kind: 'move', to: '/out\u200B' }) },
      ended(1, { outcome: { kind: 'failed', error: error('No') }, errors: [{ path: 'd\u202Ex', error: error('Denied') }] }),
    );
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    expect(screen.getByText(/a⟨U\+202E⟩fdp\.exe/)).toBeInTheDocument();
    expect(screen.getByText('→ /out⟨U+200B⟩')).toHaveAttribute('title', '/out⟨U+200B⟩');
    expect(screen.getByRole('button', { name: 'Dismiss a⟨U+202E⟩fdp.exe' })).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: /^Failed/ }));
    expect(screen.getByText('d⟨U+202E⟩x')).toBeInTheDocument();
  });

  it('has a row per job: label, target, progress, speed and time left', async () => {
    const { transfers, send } = await store();
    send({ kind: 'added', job: job(1, { label: '3 items', kind: 'move', to: '/backup' }) });
    send(progress(1, 50, 200));
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    expect(screen.getByText('3 items')).toBeInTheDocument();
    expect(screen.getByText('Move')).toBeInTheDocument();
    expect(screen.getByText('→ /backup')).toBeInTheDocument();
    expect(screen.getByRole('progressbar', { name: 'Progress of 3 items' })).toHaveAttribute('aria-valuenow', '25');
    expect(screen.getByText('10 MB/s · 1 min 5 s left')).toBeInTheDocument();
  });

  it('says where a job is before there is a speed to show', async () => {
    const { transfers, send } = await store();
    send({ kind: 'added', job: job(1, { phase: 'scanning' }) }, { kind: 'added', job: job(2, { phase: 'waiting', label: 'b' }) });
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    expect(screen.getByText('Scanning…')).toBeInTheDocument();
    expect(screen.getByText('Waiting for your answer')).toBeInTheDocument();
  });

  it('Pause becomes Resume while the job is paused, and each sends its action', async () => {
    const { transfers, send, calls } = await store();
    send({ kind: 'added', job: job(1) });
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    await fireEvent.click(screen.getByRole('button', { name: 'Pause a.pdf' }));
    send({ kind: 'paused', id: 1, paused: true });
    expect(await screen.findByText('Paused')).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Resume a.pdf' }));

    expect(calls('ops_control')).toEqual([
      { job: 1, action: 'pause' },
      { job: 1, action: 'resume' },
    ]);
  });

  it('Cancel cancels that job only', async () => {
    const { transfers, send, calls } = await store();
    send({ kind: 'added', job: job(1) }, { kind: 'added', job: job(2, { label: 'b.txt' }) });
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    await fireEvent.click(screen.getByRole('button', { name: 'Cancel b.txt' }));

    expect(calls('ops_control')).toEqual([{ job: 2, action: 'cancel' }]);
  });

  it('a finished job shows how it ended, and × dismisses it', async () => {
    const { transfers, send } = await store();
    send({ kind: 'added', job: job(1) }, ended(1, { skipped: 2 }));
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    expect(screen.getByText('Done · 2 skipped')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Pause a.pdf' })).not.toBeInTheDocument();
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '100');

    await fireEvent.click(screen.getByRole('button', { name: 'Dismiss a.pdf' }));
    expect(screen.queryByText('a.pdf')).not.toBeInTheDocument();
    expect(screen.getByText('No transfers')).toBeInTheDocument();
  });

  it('a failed job shows its errors on expand', async () => {
    const { transfers, send } = await store();
    const error = (message: string) => ({ code: 'io' as const, message });
    send(
      { kind: 'added', job: job(1) },
      ended(1, {
        outcome: { kind: 'failed', error: error('The target disk is full') },
        errors: [{ path: 'dir/x.bin', error: error('Permission denied') }],
        moreErrors: 3,
      }),
    );
    render(TransfersPanel, { transfers, onClose: vi.fn() });
    expect(screen.queryByText('Permission denied')).not.toBeInTheDocument();

    const toggle = screen.getByRole('button', { name: 'Failed · 4 errors' });
    await fireEvent.click(toggle);

    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('The target disk is full')).toBeInTheDocument();
    expect(screen.getByText('dir/x.bin')).toBeInTheDocument();
    expect(screen.getByText(/Permission denied/)).toBeInTheDocument();
    expect(screen.getByText('and 3 more')).toBeInTheDocument();

    await fireEvent.click(toggle);
    expect(screen.queryByText('Permission denied')).not.toBeInTheDocument();
  });

  it('closes on Escape, and on a click outside but not on the chip that opens it', async () => {
    const { transfers } = await store();
    const onClose = vi.fn();
    render(TransfersPanel, { transfers, onClose });
    const panel = screen.getByRole('region', { name: 'Transfers' });
    expect(panel).toHaveFocus();

    await fireEvent.click(within(panel).getByText('No transfers'));
    expect(onClose).not.toHaveBeenCalled();

    await fireEvent.keyDown(panel, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);

    const opener = document.body.appendChild(document.createElement('button'));
    opener.dataset.menuOpener = '';
    await fireEvent.click(opener);
    expect(onClose).toHaveBeenCalledTimes(1);
    opener.remove();

    await fireEvent.click(document.body);
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});

describe('TransfersPanel file tools', () => {
  it('names each kind of job, and says where a split, a combine, a pack or an add puts its output', async () => {
    const { transfers, send } = await store();
    send(
      { kind: 'added', job: job(1, { kind: 'checksum', label: 'a.iso', to: '/data' }) },
      { kind: 'added', job: job(2, { kind: 'verifyChecksum', label: 'a.iso.sha256', to: '/data' }) },
      { kind: 'added', job: job(3, { kind: 'split', label: 'movie.mkv', to: '/backup' }) },
      { kind: 'added', job: job(4, { kind: 'combine', label: 'movie.mkv.001', to: '/restore' }) },
      { kind: 'added', job: job(5, { kind: 'attributes', label: '3 items', to: '/data' }) },
      { kind: 'added', job: job(6, { kind: 'pack', label: 'photos', to: '/backup/photos.7z' }) },
      { kind: 'added', job: job(7, { kind: 'addToArchive', label: 'notes.txt', to: '/backup/old.zip' }) },
      { kind: 'added', job: job(8, { kind: 'rename', label: '3 items', to: '/data' }) },
    );
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    const kinds = ['Checksum', 'Verify', 'Split', 'Combine', 'Attributes', 'Pack', 'Add to archive', 'Rename'];
    expect(kinds.map((kind) => screen.getByText(kind))).toHaveLength(8);
    expect(screen.getByText('→ /backup/photos.7z')).toBeInTheDocument();
    expect(screen.getByText('→ /backup/old.zip')).toBeInTheDocument();
    expect(screen.getByText('→ /backup')).toBeInTheDocument();
    expect(screen.getByText('→ /restore')).toBeInTheDocument();
    expect(screen.queryByText('→ /data')).not.toBeInTheDocument(); // a rename stays in its folder
  });

  it('says where a tool is before there is a speed to show', async () => {
    const { transfers, send } = await store();
    send(
      { kind: 'added', job: job(1, { kind: 'checksum', phase: 'hashing', label: 'a' }) },
      { kind: 'added', job: job(2, { kind: 'split', phase: 'splitting', label: 'b' }) },
      { kind: 'added', job: job(3, { kind: 'combine', phase: 'combining', label: 'c' }) },
      { kind: 'added', job: job(4, { kind: 'attributes', phase: 'changing', label: 'd' }) },
      { kind: 'added', job: job(5, { kind: 'pack', phase: 'packing', label: 'e' }) },
      { kind: 'added', job: job(6, { kind: 'rename', phase: 'renaming', label: 'f' }) },
      { kind: 'added', job: job(7, { kind: 'sync', phase: 'removing', label: 'g' }) },
    );
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    for (const text of ['Hashing…', 'Splitting…', 'Combining…', 'Changing…', 'Packing…', 'Removing…', 'Moving…']) {
      expect(screen.getByText(text)).toBeInTheDocument();
    }
  });

  it('a finished check says how many files were right, and how many were not', async () => {
    const { transfers, send } = await store();
    const verify = job(1, { kind: 'verifyChecksum', label: 'a.iso.sha256' });
    const error = (message: string) => ({ code: 'io' as const, message });
    send(
      { kind: 'added', job: verify },
      counted(1, 10),
      ended(1, {
        errors: [{ path: 'x.bin', error: error('the checksum does not match') }],
        moreErrors: 1,
      }),
    );
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    expect(screen.getByText('Done · 8 ok · 2 errors')).toBeInTheDocument();
  });

  it('a check that found nothing wrong says so', async () => {
    const { transfers, send } = await store();
    send({ kind: 'added', job: job(1, { kind: 'verifyChecksum' }) }, counted(1, 3), ended(1));
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    expect(screen.getByText('Done · 3 ok')).toBeInTheDocument();
  });

  it('other jobs do not count files as right', async () => {
    const { transfers, send } = await store();
    send({ kind: 'added', job: job(1, { kind: 'checksum' }) }, counted(1, 3), ended(1));
    render(TransfersPanel, { transfers, onClose: vi.fn() });

    expect(screen.getByText('Done')).toBeInTheDocument();
  });
});

describe('TransfersChip', () => {
  it('is not there while there are no jobs', async () => {
    const { transfers } = await store();
    render(TransfersChip, { transfers, open: false, onToggle: vi.fn() });

    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('shows the number of jobs and how far they are, and opens the panel', async () => {
    const { transfers, send } = await store();
    const onToggle = vi.fn();
    render(TransfersChip, { transfers, open: false, onToggle });
    send({ kind: 'added', job: job(1) }, { kind: 'added', job: job(2) });
    send(progress(1, 30, 100), progress(2, 60, 100));

    const chip = await screen.findByRole('button', { name: 'Transfers (Ctrl+J)' });
    expect(chip).toHaveTextContent('⇅ 2 · 45 %');
    expect(chip).toHaveAttribute('aria-expanded', 'false');

    await fireEvent.click(chip);
    expect(onToggle).toHaveBeenCalledOnce();
  });

  it('shows just the count while no job knows its size yet', async () => {
    const { transfers, send } = await store();
    send({ kind: 'added', job: job(1, { phase: 'scanning' }) });
    render(TransfersChip, { transfers, open: true, onToggle: vi.fn() });

    expect(screen.getByRole('button', { name: /Transfers/ })).toHaveTextContent(/^⇅ 1$/);
    expect(screen.getByRole('button', { name: /Transfers/ })).toHaveAttribute('aria-expanded', 'true');
  });
});
