import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import { Listing } from '$lib/panes/listing.svelte';

import { entry, fakeBackend } from '../../../tests/panes';
import Pane from './Pane.svelte';

const TOTAL = 100_000;

async function huge() {
  const backend = fakeBackend();
  const listing = new Listing(backend.ipc, 'left');
  await listing.open('/big');
  backend.last().emit({
    kind: 'start',
    path: '/big',
    parent: '/',
    crumbs: [
      { name: '/', path: '/' },
      { name: 'big', path: '/big' },
    ],
    total: TOTAL,
  });
  backend.last().emit({
    kind: 'batch',
    offset: 0,
    entries: [entry('sub', 'dir'), ...Array.from({ length: 199 }, (_, i) => entry(`f${i}.txt`, 'file', { size: 10 }))],
  });

  render(Pane, { label: 'Left pane', listing, active: true, onActivate: vi.fn() });
  return { backend, listing };
}

describe('Pane', () => {
  it('renders only the rows in view of a 100 000-entry folder', async () => {
    await huge();

    const rows = document.querySelectorAll('.row');
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeLessThan(60);
    expect(screen.getByText('..')).toBeInTheDocument();
    expect(screen.getByText('sub')).toBeInTheDocument();
    expect((document.querySelector('.spacer') as HTMLElement).style.height).toBe(`${(TOTAL + 1) * 24}px`);
  });

  it('a batch that arrives after the first render fills the rows in place', async () => {
    const { backend } = await huge();
    const viewport = document.querySelector('.viewport') as HTMLElement;
    viewport.scrollTop = 5_000 * 24;
    await fireEvent.scroll(viewport);
    expect(screen.queryByText('late')).not.toBeInTheDocument();

    backend.last().emit({ kind: 'batch', offset: 4_999, entries: [entry('late.txt')] }); // row 5 000 after `..`
    await tick();

    expect(screen.getByText('late')).toBeInTheDocument();
  });

  it('scrolling far down renders the rows there', async () => {
    await huge();
    const viewport = document.querySelector('.viewport') as HTMLElement;

    viewport.scrollTop = 150 * 24;
    await fireEvent.scroll(viewport);

    expect(screen.getByText('f140')).toBeInTheDocument(); // row 150 = `..`, `sub`, then f0…
    expect(screen.queryByText('sub')).not.toBeInTheDocument();
  });

  it('double-clicking a folder opens it, `..` opens the parent, a crumb opens its folder', async () => {
    const { backend } = await huge();

    await fireEvent.dblClick(screen.getByText('sub'));
    expect(backend.last().args).toMatchObject({ path: '/big', child: 'sub' });

    await fireEvent.dblClick(screen.getByText('..'));
    expect(backend.last().args).toMatchObject({ path: '/', child: null });

    await fireEvent.click(screen.getByRole('button', { name: 'big' }));
    expect(backend.last().args).toMatchObject({ path: '/big', child: null });
  });

  it('shows the summary when done and the error when the folder cannot be read', async () => {
    const { backend } = await huge();

    backend.last().emit({ kind: 'done', dirs: 1, files: 99_999, bytes: 2048 });
    expect(await screen.findByText('1 folder · 99,999 files · 2.0 KB')).toBeInTheDocument();

    backend.last().emit({ kind: 'error', error: { code: 'notFound', message: 'This folder does not exist.' } });
    expect(await screen.findByRole('alert')).toHaveTextContent('This folder does not exist.');
  });
});
