import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

vi.mock('./pdf', () => ({ openPdf: vi.fn() }));

import { FakeFile, openedSession } from '../../../tests/view';
import { openPdf, type PdfDocument } from './pdf';
import PdfView from './PdfView.svelte';

/** A three-page document whose drawing is a mock. */
type Fake = { pages: number; render: Mock<PdfDocument['render']>; destroy: Mock<PdfDocument['destroy']> };

const document = (): Fake => ({
  pages: 3,
  render: vi.fn<PdfDocument['render']>().mockResolvedValue(undefined),
  destroy: vi.fn<PdfDocument['destroy']>(),
});

const sizes = {
  width: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientWidth'),
  height: Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight'),
};

// Nothing is laid out here: the room for the page is 624 x 424 inside its margin.
beforeEach(() => {
  vi.mocked(openPdf).mockReset();
  Object.defineProperty(HTMLElement.prototype, 'clientWidth', { configurable: true, get: () => 648 });
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', { configurable: true, get: () => 448 });
});

afterEach(() => {
  for (const [name, original] of [['clientWidth', sizes.width], ['clientHeight', sizes.height]] as const) {
    if (original) {
      Object.defineProperty(HTMLElement.prototype, name, original);
    } else {
      delete (HTMLElement.prototype as unknown as Record<string, unknown>)[name];
    }
  }
});

async function open(doc: Fake = document()) {
  vi.mocked(openPdf).mockResolvedValue(doc);
  const world = await openedSession(new FakeFile('%PDF-'), () => ({ kind: 'pdf' }));
  const view = render(PdfView, { session: world.session });

  return { ...world, view, doc };
}

const pageOf = () => screen.getByRole('status');

describe('PdfView', () => {
  it('opens the session`s file and draws the first page to fit the room', async () => {
    const { doc, session } = await open();

    expect(await screen.findByText('Page 1 of 3')).toBeInTheDocument();

    expect(vi.mocked(openPdf).mock.calls[0][0]).toBe(session);
    await vi.waitFor(() => expect(doc.render).toHaveBeenCalledTimes(1));
    expect(doc.render.mock.calls[0][0]).toBe(1);
    expect(doc.render.mock.calls[0][2]).toEqual({ width: 624, height: 424 });
  });

  it('the buttons and PageUp / PageDown turn the page, and stop at the ends', async () => {
    const { doc } = await open();
    await screen.findByText('Page 1 of 3');
    const prev = screen.getByRole('button', { name: 'Previous page' });
    const next = screen.getByRole('button', { name: 'Next page' });
    expect(prev).toBeDisabled();

    await fireEvent.click(next);
    expect(pageOf()).toHaveTextContent('Page 2 of 3');
    await fireEvent.keyDown(next, { key: 'PageDown' });
    expect(pageOf()).toHaveTextContent('Page 3 of 3');
    expect(next).toBeDisabled();
    await fireEvent.keyDown(next, { key: 'PageDown' });
    expect(pageOf()).toHaveTextContent('Page 3 of 3');
    await fireEvent.keyDown(next, { key: 'PageUp' });
    await fireEvent.click(prev);
    expect(pageOf()).toHaveTextContent('Page 1 of 3');
    await fireEvent.keyDown(prev, { key: 'PageUp' });
    expect(pageOf()).toHaveTextContent('Page 1 of 3');

    await vi.waitFor(() => expect(doc.render.mock.calls.map((call) => call[0])).toEqual([1, 2, 3, 2, 1]));
  });

  it('says so when the file is not a PDF it can open', async () => {
    vi.mocked(openPdf).mockRejectedValue(new Error('Invalid PDF structure'));
    const world = await openedSession(new FakeFile('nope'), () => ({ kind: 'pdf' }));
    render(PdfView, { session: world.session });

    expect(await screen.findByRole('alert')).toHaveTextContent('This PDF cannot be shown here');
  });

  it('says so when a page cannot be drawn', async () => {
    const doc = document();
    doc.render.mockRejectedValue(new Error('bad page'));
    await open(doc);

    expect(await screen.findByRole('alert')).toHaveTextContent('This PDF cannot be shown here');
  });

  it('frees the document, and stops its reads, when the view goes', async () => {
    const { doc, view } = await open();
    await screen.findByText('Page 1 of 3');
    const signal = vi.mocked(openPdf).mock.calls[0][1];
    expect(signal.aborted).toBe(false);

    view.unmount();

    expect(signal.aborted).toBe(true);
    expect(doc.destroy).toHaveBeenCalledTimes(1);
  });
});
