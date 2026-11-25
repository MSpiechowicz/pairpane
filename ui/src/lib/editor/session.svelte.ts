import type { EditDocument } from '$lib/ipc/bindings/EditDocument';
import type { EditOpened } from '$lib/ipc/bindings/EditOpened';
import type { EditSaveOptions } from '$lib/ipc/bindings/EditSaveOptions';
import type { LineEnding } from '$lib/ipc/bindings/LineEnding';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import type { EditorSurface } from './surface';

/** How a save ended: the file is written, it changed (or went) on disk since it was read, or it was refused. */
export type SaveResult = 'saved' | 'changed' | 'failed';

/** How each line ending is written for the user. */
export const ENDING_LABELS: Record<LineEnding, string> = { lf: 'LF', crlf: 'CRLF', cr: 'CR' };

/** Whether the encoding can start with a byte order mark (UTF-8 and UTF-16; the others have none). */
export const isUnicode = (encoding: string) => encoding === 'UTF-8' || encoding.startsWith('UTF-16');

/**
 * One file in the editor: what the backend read, the choices for saving it (encoding, line ending, byte order
 * mark), and whether the text differs from the file. The text itself is the surface's while one is attached;
 * the session never keeps it twice.
 */
export class EditSession {
  readonly path: string;
  readonly name: string;
  /** For an entry of an archive (read-only, for a comparison): the folder it is in, inside the archive (`path` is then the archive file). */
  readonly inside: string | undefined;

  /** `opening` until the backend answers; `undecodable` when the bytes are not text in the encoding that was tried. */
  status = $state<'opening' | 'ready' | 'undecodable'>('opening');
  /** The encoding that could not read the file (when `undecodable`, for the picker). */
  unreadable = $state('');
  readOnly = $state(false);
  mixedEndings = $state(false);
  /** The file can no longer be read as text (a reload found bytes): what is shown stays, but cannot be changed or saved. */
  unavailable = $state(false);

  /** What the file is saved in; changed by the user, it is no longer what the file was read with. */
  encoding = $state('UTF-8');
  bom = $state(false);
  lineEnding = $state<LineEnding>('lf');

  dirty = $state(false);
  saving = $state(false);
  /** Why the last save or read was refused (the backend's words; it never names a path). */
  failure = $state<string | undefined>();
  /** Whether the file that changed since it was read (the last save was refused for it) is gone altogether. */
  gone = $state(false);
  /** What the user should know about the choices they made. */
  note = $state<string | undefined>();
  cursor = $state({ line: 1, col: 1 });

  readonly #ipc: Transport;
  readonly #onDirty: (dirty: boolean) => void;
  #surface: EditorSurface | undefined;
  /** The text while no surface holds it. */
  #text = '';
  #revision = '';
  /** Counts every change of the text and of the choices: a save only clears `dirty` when none came after it began. */
  #edits = 0;

  constructor(ipc: Transport, path: string, name: string, onDirty: (dirty: boolean) => void = () => {}, inside?: string) {
    this.#ipc = ipc;
    this.path = path;
    this.name = name;
    this.#onDirty = onDirty;
    this.inside = inside;
  }

  /** Reads the file (in `encoding`, or the one the backend finds); the caller deals with an answer that is no text. */
  async open(encoding: string | null): Promise<EditOpened> {
    const opened =
      this.inside === undefined
        ? await this.#ipc.invoke('edit_open', { path: this.path, name: this.name, encoding })
        : await this.#ipc.invoke('archive_edit_open', { archive: this.path, dir: this.inside, name: this.name, encoding });

    if (opened.kind === 'text') {
      this.#load(opened.document);
    } else if (opened.kind === 'undecodable') {
      this.#unreadable(opened.encoding);
    }

    return opened;
  }

  #load(document: EditDocument) {
    this.#text = document.text;
    this.#surface?.setText(document.text);
    this.#surface?.setReadOnly(document.readOnly);
    this.#revision = document.revision;

    this.readOnly = document.readOnly;
    this.mixedEndings = document.mixedEndings;
    this.encoding = document.encoding;
    this.bom = document.bom;
    this.lineEnding = document.lineEnding;
    this.failure = undefined;
    this.note = undefined;
    this.cursor = { line: 1, col: 1 };
    this.status = 'ready';
    this.#edits += 1;
    this.#setDirty(false);
  }

  #unreadable(encoding: string) {
    if (this.status === 'ready') {
      // A file that read well stays in the editor; only the choice of encoding failed.
      this.failure = `The file is not valid ${encoding} text, or would not be saved byte for byte in it.`;
      return;
    }

    this.unreadable = encoding;
    this.status = 'undecodable';
  }

  /** Where the text is edited: the session hears of every change and move of the cursor. */
  attach(surface: EditorSurface) {
    this.#surface = surface;
    this.cursor = surface.cursor();
    surface.onChange((edited) => {
      this.cursor = surface.cursor();
      if (edited) {
        this.failure = undefined;
        this.#changed();
      }
    });
  }

  /** The surface is going away: its text is kept. */
  detach() {
    if (this.#surface !== undefined) {
      this.#text = this.#surface.text();
      this.#surface = undefined;
    }
  }

  text(): string {
    return this.#surface?.text() ?? this.#text;
  }

  setLineEnding(lineEnding: LineEnding) {
    this.lineEnding = lineEnding;
    this.#changed();
  }

  setBom(bom: boolean) {
    this.bom = bom;
    this.#changed();
  }

  /**
   * The user picked an encoding. On a clean buffer the file is read again in it (the answer is for the caller);
   * on a changed one that would lose the changes, so only the encoding the file is saved in changes.
   */
  chooseEncoding(encoding: string): Promise<EditOpened> | undefined {
    if (this.status === 'ready' && this.dirty) {
      this.encoding = encoding;
      this.note = 'Only the encoding the file is saved in changes; the text stays as it is.';
      return undefined;
    }

    return this.open(encoding);
  }

  /** What a save writes. A byte order mark is for the Unicode encodings only. */
  #options(force: boolean): EditSaveOptions {
    return {
      encoding: this.encoding,
      bom: isUnicode(this.encoding) && this.bom,
      lineEnding: this.lineEnding,
      revision: this.#revision,
      force,
    };
  }

  /** Writes the text. `force` saves over a file that changed (or went) since it was read. */
  async save(force: boolean): Promise<SaveResult> {
    // An entry of an archive is never written, whatever it says: the archive is not a folder.
    if (this.status !== 'ready' || this.readOnly || this.inside !== undefined || this.unavailable || this.saving) {
      return 'failed';
    }

    const edits = this.#edits;
    this.saving = true;
    this.failure = undefined;

    try {
      const saved = await this.#ipc.invoke('edit_save', {
        path: this.path,
        name: this.name,
        text: this.text(),
        options: this.#options(force),
      });
      if (saved.kind === 'changed') {
        this.gone = saved.gone;
        return 'changed';
      }

      this.#revision = saved.revision;
      this.mixedEndings = false;
      this.note = undefined;
      if (edits === this.#edits) {
        this.#setDirty(false);
      }

      return 'saved';
    } catch (error) {
      this.failure = IpcFailure.from(error).message;
      return 'failed';
    } finally {
      this.saving = false;
    }
  }

  /** A reload found something that is not text: the text on screen is kept as it was, locked, and `note` says why. */
  retire(note: string) {
    this.unavailable = true;
    this.readOnly = true;
    this.#surface?.setReadOnly(true);
    this.failure = note;
    this.#setDirty(false);
  }

  /** The editor is closed: what was changed is given up. */
  release() {
    this.#setDirty(false);
  }

  #changed() {
    this.#edits += 1;
    this.#setDirty(true);
  }

  #setDirty(dirty: boolean) {
    if (this.dirty !== dirty) {
      this.dirty = dirty;
      this.#onDirty(dirty);
    }
  }
}
