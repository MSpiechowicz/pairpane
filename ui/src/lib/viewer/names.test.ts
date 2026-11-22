// A file name that reorders text (`invoice<RLO>fdp.exe`) is written out where the viewer shows it, in its title,
// its toolbar, its quick view and what the views say of it aloud; what goes to the backend is the real name.
import { render, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

vi.mock('./pdf', () => ({ openPdf: vi.fn() }));

import type { ArchiveListing } from '$lib/ipc/bindings/ArchiveListing';
import type { ViewKind } from '$lib/ipc/bindings/ViewKind';

import type { Answers } from '../../../tests/app';
import { FakeFile, numbered, openedSession } from '../../../tests/view';
import ArchiveView from './ArchiveView.svelte';
import HexView from './HexView.svelte';
import ImageView from './ImageView.svelte';
import MarkdownView from './MarkdownView.svelte';
import { openPdf } from './pdf';
import PdfView from './PdfView.svelte';
import QuickView from './QuickView.svelte';
import ViewerOverlay from './ViewerOverlay.svelte';

const NAME = 'invoice‮fdp.exe';
const SHOWN = 'invoice⟨U+202E⟩fdp.exe';

/** A session on a file with that name. */
function opened(kind: ViewKind = { kind: 'text' }, extra: Answers = {}, notice?: string) {
  return openedSession(new FakeFile(numbered(5)), () => kind, extra, NAME, notice);
}

describe('the viewer shows a file name with its characters that reorder text written out', () => {
  it('in the overlay: its title, its toolbar and the text view`s name', async () => {
    const { session } = await opened();
    render(ViewerOverlay, { session, onClose: vi.fn() });

    expect(session.name).toBe(NAME);
    expect(screen.getByRole('dialog', { name: SHOWN })).toBeInTheDocument();
    expect(screen.getByText(SHOWN, { selector: '.overlay-bar-name' })).toBeInTheDocument();
    expect(await screen.findByRole('textbox', { name: `Text of ${SHOWN}` })).toBeInTheDocument();
  });

  it('in the overlay: what the viewer notes about the file', async () => {
    const { session } = await opened(undefined, {}, 'Showing a\u202Etxt.exe as text');
    render(ViewerOverlay, { session, onClose: vi.fn() });

    expect(screen.getByText('Showing a⟨U+202E⟩txt.exe as text')).toBeInTheDocument();
  });

  it('in the quick view', async () => {
    const { session } = await opened();
    const viewer = { quickShown: session, under: undefined, follow: vi.fn(), unfollow: vi.fn() };
    render(QuickView, { viewer: viewer as never });

    expect(screen.getByText(SHOWN, { selector: 'header' })).toBeInTheDocument();
  });

  it('in the hex view', async () => {
    const { session } = await opened();
    render(HexView, { session });

    expect(await screen.findByRole('textbox', { name: `Bytes of ${SHOWN}` })).toBeInTheDocument();
  });

  it('in the rendered Markdown', async () => {
    const { session } = await opened({ kind: 'markdown' }, { view_markdown: () => ({ blocks: [], truncated: false }) });
    render(MarkdownView, { session });

    expect(await screen.findByRole('document', { name: `Rendered ${SHOWN}` })).toBeInTheDocument();
  });

  it('in the PDF view', async () => {
    vi.mocked(openPdf).mockResolvedValue({ pages: 1, render: vi.fn().mockResolvedValue(undefined), destroy: vi.fn() });
    const { session } = await opened({ kind: 'pdf' });
    render(PdfView, { session });

    expect(await screen.findByRole('document', { name: `PDF ${SHOWN}` })).toBeInTheDocument();
  });

  it('in the image view', async () => {
    const created = vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:image-1');
    const { session } = await opened({ kind: 'image', mime: 'image/png' });
    render(ImageView, { session });

    expect(await screen.findByRole('img', { name: SHOWN })).toBeInTheDocument();
    created.mockRestore();
  });

  it('in the archive view: its region and each entry`s name', async () => {
    const listing: ArchiveListing = {
      format: 'zip',
      truncated: false,
      entries: [
        { name: NAME, size: 10, dir: false, modified: null },
        { name: 'a‎b/', size: 0, dir: true, modified: null },
        { name: 'c‮d', size: 0, dir: true, modified: null },
      ],
    };
    const { session } = await opened({ kind: 'archive', format: 'zip' }, { view_archive: () => listing });
    render(ArchiveView, { session });

    const region = await screen.findByRole('region', { name: `Contents of ${SHOWN}` });
    const names = within(region).getAllByRole('cell').filter((cell) => cell.classList.contains('name'));
    expect(names.map((cell) => cell.textContent)).toEqual([SHOWN, 'a⟨U+200E⟩b/', 'c⟨U+202E⟩d/']);
  });
});
