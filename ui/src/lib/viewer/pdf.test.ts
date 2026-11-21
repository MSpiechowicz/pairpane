import { beforeEach, describe, expect, it, vi } from 'vitest';

// pdf.js itself is not loaded here: a document whose pages are mocks, and the range transport it is given.
vi.mock('pdfjs-dist', () => {
  class PDFDataRangeTransport {
    onDataRange = vi.fn();

    constructor(
      readonly length: number,
      readonly initialData: Uint8Array,
    ) {}
  }

  class RenderingCancelledException extends Error {}

  return {
    GlobalWorkerOptions: { workerPort: {} },
    PDFDataRangeTransport,
    RenderingCancelledException,
    getDocument: vi.fn(),
  };
});

import { getDocument, RenderingCancelledException } from 'pdfjs-dist';

import { FakeFile, openedSession } from '../../../tests/view';
import { openPdf } from './pdf';

const CONTENT = '%PDF-1.4 0123456789';

type Transport = { requestDataRange(begin: number, end: number): void; onDataRange: ReturnType<typeof vi.fn> };

/** A page that draws until it is cancelled. */
function page() {
  const render = vi.fn(() => {
    let reject = (_: unknown) => {};
    const promise = new Promise<void>((_, fail) => (reject = fail));

    return { promise, cancel: vi.fn(() => reject(new RenderingCancelledException('cancelled'))) };
  });

  return { getViewport: () => ({ width: 100, height: 200 }), render };
}

/** A document of two pages (`getPage` is the test's to answer) opened over a fake file whose reads can be held. */
async function open() {
  const getPage = vi.fn<(number: number) => Promise<ReturnType<typeof page>>>();
  vi.mocked(getDocument).mockImplementation(
    () => ({ promise: Promise.resolve({ numPages: 2, getPage }), destroy: () => Promise.resolve() }) as never,
  );

  const state = { hold: false, busy: 0 };
  const held: (() => void)[] = [];
  const file = new FakeFile(CONTENT);
  const world = await openedSession(file, () => ({ kind: 'pdf' }), {
    view_bytes: (args: { offset: number; length: number }) => {
      const bytes = file.bytes.slice(args.offset, args.offset + args.length).buffer;
      if (state.busy > 0) {
        state.busy -= 1;

        return Promise.reject({ code: 'busy', message: 'the viewer is busy' });
      }

      return state.hold ? new Promise((answer) => held.push(() => answer(bytes))) : bytes;
    },
  });

  const onError = vi.fn();
  const document = await openPdf(world.session, new AbortController().signal, onError);
  const transport = vi.mocked(getDocument).mock.calls[0][0] as unknown as { range: Transport };

  return { ...world, document, transport: transport.range, state, held, getPage, onError };
}

beforeEach(() => vi.mocked(getDocument).mockReset());

describe('openPdf', () => {
  it('reads the ranges pdf.js asks for one after the other', async () => {
    const { transport, state, held } = await open();
    state.hold = true;

    transport.requestDataRange(0, 4);
    transport.requestDataRange(4, 8);
    await vi.waitFor(() => expect(held).toHaveLength(1));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(held).toHaveLength(1);

    held[0]();
    await vi.waitFor(() => expect(held).toHaveLength(2));
    held[1]();

    await vi.waitFor(() => expect(transport.onDataRange).toHaveBeenCalledTimes(2));
    expect(transport.onDataRange.mock.calls.map(([offset, bytes]) => [offset, bytes.length])).toEqual([
      [0, 4],
      [4, 4],
    ]);
  });

  it('asks again for a range the backend refused as busy', async () => {
    const { transport, state, onError } = await open();
    state.busy = 2;

    transport.requestDataRange(0, 4);

    await vi.waitFor(() => expect(transport.onDataRange).toHaveBeenCalledTimes(1));
    expect(onError).not.toHaveBeenCalled();
  });

  it('reports a range that stays refused, and goes on with the next one', async () => {
    const { transport, state, onError } = await open();
    state.busy = 4;

    transport.requestDataRange(0, 4);
    await vi.waitFor(() => expect(onError).toHaveBeenCalledTimes(1));
    expect(onError.mock.calls[0][0]).toMatchObject({ code: 'busy' });

    transport.requestDataRange(4, 8);
    await vi.waitFor(() => expect(transport.onDataRange).toHaveBeenCalledTimes(1));
  });

  it('draws only the latest page when pages are asked for while the first is on its way', async () => {
    const { document, getPage } = await open();
    const pages = [page(), page()];
    const answers: (() => void)[] = [];
    getPage.mockImplementation((number) => new Promise((answer) => answers.push(() => answer(pages[number - 1]))));
    const canvas = window.document.createElement('canvas');

    const first = document.render(1, canvas, { width: 100, height: 100 });
    void document.render(2, canvas, { width: 100, height: 100 });
    answers.forEach((answer) => answer());
    await first;

    expect(pages[0].render).not.toHaveBeenCalled();
    expect(pages[1].render).toHaveBeenCalledTimes(1);
  });

  it('cancels the drawing that goes on when another page is asked for, and that is no failure', async () => {
    const { document, getPage } = await open();
    const pages = [page(), page()];
    getPage.mockImplementation((number) => Promise.resolve(pages[number - 1]));
    const canvas = window.document.createElement('canvas');

    const first = document.render(1, canvas, { width: 100, height: 100 });
    await vi.waitFor(() => expect(pages[0].render).toHaveBeenCalledTimes(1));
    void document.render(2, canvas, { width: 100, height: 100 });

    await expect(first).resolves.toBeUndefined();
    expect(pages[0].render.mock.results[0].value.cancel).toHaveBeenCalledTimes(1);
    await vi.waitFor(() => expect(pages[1].render).toHaveBeenCalledTimes(1));
  });
});
