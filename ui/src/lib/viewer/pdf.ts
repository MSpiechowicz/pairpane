import type { PDFDocumentLoadingTask, PDFDocumentProxy, PDFDataRangeTransport, RenderTask } from 'pdfjs-dist';

import { CHUNK, chunks } from './bytes';
import type { ViewSession } from './session.svelte';

/** A PDF opened for viewing: its page count and the drawing of one page. */
export interface PdfDocument {
  readonly pages: number;
  /**
   * Draws page `page` (from 1) on the canvas, as large as fits `box`; an earlier drawing, going or not yet begun,
   * gives way to it.
   */
  render(page: number, canvas: HTMLCanvasElement, box: { width: number; height: number }): Promise<void>;
  /** Frees the document and stops what it is doing. */
  destroy(): void;
}

type Pdfjs = typeof import('pdfjs-dist');

let library: Promise<Pdfjs> | undefined;

/** pdf.js, loaded on first use; its worker is made once, as a module of the app's own (the CSP allows no other). */
function pdfjs(): Promise<Pdfjs> {
  library ??= import('pdfjs-dist').then((lib) => {
    if (!lib.GlobalWorkerOptions.workerPort) {
      lib.GlobalWorkerOptions.workerPort = new Worker(new URL('pdfjs-dist/build/pdf.worker.min.mjs', import.meta.url), {
        type: 'module',
      });
    }

    return lib;
  });

  return library;
}

/**
 * The last document still being destroyed: pdf.js refuses a new document on the shared worker until the one before
 * it is gone, so {@link openPdf} waits for this.
 */
let closing: Promise<unknown> = Promise.resolve();

const released = new WeakSet<PDFDocumentLoadingTask>();

/** Destroys the task (once, however often it is asked). */
function release(task: PDFDocumentLoadingTask) {
  if (!released.has(task)) {
    released.add(task);
    closing = task.destroy().catch(() => {});
  }
}

/**
 * The ranges of the file pdf.js asks for, read through `view_bytes` in pieces of at most 4 MiB. pdf.js asks for many
 * at once, but they are read one after the other: the backend has few read threads, shared with the other views.
 */
function rangeTransport(
  lib: Pdfjs,
  session: ViewSession,
  initial: Uint8Array,
  signal: AbortSignal,
  onError: (reason: unknown) => void,
): PDFDataRangeTransport {
  return new (class extends lib.PDFDataRangeTransport {
    #queue: Promise<void> = Promise.resolve();

    requestDataRange(begin: number, end: number) {
      const last = Math.min(end, this.length);
      this.#queue = this.#queue.then(() => this.#read(begin, last));
    }

    async #read(begin: number, end: number) {
      try {
        for await (const { offset, bytes } of chunks(session, begin, end, signal)) {
          this.onDataRange(offset, bytes);
        }
      } catch (reason) {
        if (!signal.aborted) {
          onError(reason);
        }
      }
    }
  })(session.size, initial);
}

/**
 * Opens the session's file as a PDF. Only the parts pdf.js asks for are read, no ahead of time and not by a stream;
 * there is no CMap, font or wasm file to load, so a few kinds of image and some CJK text do not draw.
 * `onError` hears of a read that fails once the document is open. Rejects when the file is not a PDF it can open.
 */
export async function openPdf(
  session: ViewSession,
  signal: AbortSignal,
  onError: (reason: unknown) => void,
): Promise<PdfDocument> {
  const lib = await pdfjs();
  await closing;
  signal.throwIfAborted();

  const first = (await chunks(session, 0, Math.min(session.size, CHUNK), signal).next()).value;
  if (first === undefined) {
    throw new Error('The file is empty');
  }

  const task: PDFDocumentLoadingTask = lib.getDocument({
    range: rangeTransport(lib, session, first.bytes, signal, onError),
    useWasm: false,
    disableAutoFetch: true,
    disableStream: true,
  });
  signal.addEventListener('abort', () => release(task), { once: true });

  const document: PDFDocumentProxy = await task.promise;
  signal.throwIfAborted();

  return new Pages(lib, document, task);
}

class Pages implements PdfDocument {
  readonly pages: number;
  readonly #lib: Pdfjs;
  readonly #document: PDFDocumentProxy;
  readonly #task: PDFDocumentLoadingTask;
  #drawing: RenderTask | undefined;
  /** Bumped by every drawing asked for (and by the end of the document): only the latest one gets to draw. */
  #latest = 0;

  constructor(lib: Pdfjs, document: PDFDocumentProxy, task: PDFDocumentLoadingTask) {
    this.#lib = lib;
    this.#document = document;
    this.#task = task;
    this.pages = document.numPages;
  }

  async render(page: number, canvas: HTMLCanvasElement, box: { width: number; height: number }) {
    this.#latest += 1;
    const ticket = this.#latest;
    this.#drawing?.cancel();

    const proxy = await this.#document.getPage(page);
    if (ticket !== this.#latest) {
      return; // a newer drawing was asked for (or the document is gone) while the page was on its way
    }

    const natural = proxy.getViewport({ scale: 1 });
    const fit = Math.min(box.width / natural.width, box.height / natural.height);
    const scale = Number.isFinite(fit) && fit > 0 ? fit : 1;
    const ratio = window.devicePixelRatio || 1;
    const viewport = proxy.getViewport({ scale: scale * ratio });

    canvas.width = Math.floor(viewport.width);
    canvas.height = Math.floor(viewport.height);
    canvas.style.width = `${Math.floor(viewport.width / ratio)}px`;
    canvas.style.height = `${Math.floor(viewport.height / ratio)}px`;

    const drawing = proxy.render({ canvas, viewport });
    this.#drawing = drawing;

    try {
      await drawing.promise;
    } catch (reason) {
      if (!(reason instanceof this.#lib.RenderingCancelledException)) {
        throw reason;
      }
    }
  }

  destroy() {
    this.#latest += 1;
    this.#drawing?.cancel();
    release(this.#task);
  }
}
