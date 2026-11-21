import type { ViewKind } from '$lib/ipc/bindings/ViewKind';
import type { ViewOpened } from '$lib/ipc/bindings/ViewOpened';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { DEFAULT_ENCODING } from './encodings';

/** What the toolbar asks to see: the file's own view (`auto`), its text or its bytes. */
export type ViewMode = 'auto' | 'text' | 'hex';

/** What is drawn: text, bytes, or the preview that belongs to the kind (an image, a page, a rendering, a listing). */
export type Shown = 'text' | 'hex' | 'preview';

/** The kinds that have a preview of their own; text and anything unknown are shown as text or bytes. */
const PREVIEWED: readonly ViewKind['kind'][] = ['markdown', 'image', 'pdf', 'audio', 'video', 'archive'];

/**
 * One viewer session on one file: the backend session it holds open, what the file is, and the choices the user
 * made for it (mode, encoding, wrapping). It opens at once and closes exactly once, also when it is closed before
 * the backend has answered. Every kind can be seen as text and as bytes, whatever its own view is.
 */
export class ViewSession {
  readonly name: string;
  readonly path: string;
  readonly ipc: Transport;
  /** What the user should know about why this file is shown (it was too large to edit). */
  readonly notice: string | undefined;

  status = $state<'opening' | 'ready' | 'failed'>('opening');
  /** Why the file could not be opened; the backend's own words (it never names a path). */
  error = $state<string | undefined>();
  opened = $state.raw<ViewOpened | undefined>();

  mode = $state<ViewMode>('auto');
  encoding = $state(DEFAULT_ENCODING);
  wrap = $state(false);
  /** The byte offset at the top of the text or hex view; the other one starts from it, and so does a search. */
  position = $state(0);

  #closed = false;

  constructor(ipc: Transport, path: string, name: string, notice?: string) {
    this.ipc = ipc;
    this.path = path;
    this.name = name;
    this.notice = notice;
    void this.#open();
  }

  async #open() {
    try {
      const opened = await this.ipc.invoke('view_open', { path: this.path, name: this.name });
      if (this.#closed) {
        this.#release(opened.session);
        return;
      }

      this.opened = opened;
      this.encoding = opened.encoding === '' ? DEFAULT_ENCODING : opened.encoding;
      this.status = 'ready';
    } catch (error) {
      if (!this.#closed) {
        this.error = IpcFailure.from(error).message;
        this.status = 'failed';
      }
    }
  }

  #release(session: number) {
    this.ipc.invoke('view_close', { session }).catch(() => {});
  }

  /** Ends the session on the backend (when it was opened); a second call does nothing. */
  close() {
    if (this.#closed) {
      return;
    }

    this.#closed = true;
    if (this.opened !== undefined) {
      this.#release(this.opened.session);
    }
  }

  get id(): number | undefined {
    return this.opened?.session;
  }

  get size(): number {
    return this.opened?.size ?? 0;
  }

  get kind(): ViewKind['kind'] | undefined {
    return this.opened?.kind.kind;
  }

  /** Whether the kind has a preview of its own, which the modes `text` and `hex` are alternatives to. */
  get previewed(): boolean {
    return this.kind !== undefined && PREVIEWED.includes(this.kind);
  }

  /** The modes the toolbar offers: the preview only where there is one. */
  get modes(): readonly ViewMode[] {
    return this.previewed ? ['auto', 'text', 'hex'] : ['text', 'hex'];
  }

  /** What is drawn: `auto` is the preview, the text of a text file and the bytes of anything binary. */
  get shown(): Shown {
    if (this.mode === 'text' || this.mode === 'hex') {
      return this.mode;
    }

    if (this.previewed) {
      return 'preview';
    }

    return this.kind === 'binary' ? 'hex' : 'text';
  }

  /** The mode the toolbar marks: what is drawn, where there is no preview to name. */
  get marked(): ViewMode {
    return this.shown === 'preview' ? 'auto' : this.shown;
  }

  /** Whether the file's bytes are read as text in the encoding the user can change. */
  get decoded(): boolean {
    return this.shown === 'text' || (this.shown === 'preview' && this.kind === 'markdown');
  }
}
