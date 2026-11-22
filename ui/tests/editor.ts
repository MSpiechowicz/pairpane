// Test helpers for the editor: a surface that is a plain textarea (CodeMirror is not what these tests are about),
// and the answers of a fake backend for a file that opens and saves.
import { fireEvent, render, screen } from '@testing-library/svelte';
import { vi } from 'vitest';

import { Editor } from '$lib/editor/editor.svelte';
import type { EditDocument } from '$lib/ipc/bindings/EditDocument';
import type { EditSaveOptions } from '$lib/ipc/bindings/EditSaveOptions';
import type { EditorSurface, SurfaceOptions } from '$lib/editor/surface';

import type { CommandName } from '$lib/ipc/commands';

import type { Answers } from './app';
import EditorHost from './EditorHost.svelte';
import { fakeBackend } from './panes';

/** Every surface made, the last one is the one on screen. */
export const surfaces: FakeSurface[] = [];

/** The editing widget as a textarea: typing in it is an `input` event, like the real one's changes. */
export class FakeSurface implements EditorSurface {
  readonly area: HTMLTextAreaElement;
  searches: boolean[] = [];
  #listener: (edited: boolean) => void = () => {};

  constructor(options: SurfaceOptions) {
    this.area = document.createElement('textarea');
    this.area.setAttribute('aria-label', `Text of ${options.name}`);
    this.area.value = options.text;
    this.area.setSelectionRange(0, 0);
    this.area.readOnly = options.readOnly;
    this.area.addEventListener('input', () => this.#listener(true));
    this.area.addEventListener('keyup', () => this.#listener(false));
    options.parent.append(this.area);
    surfaces.push(this);
  }

  text = () => this.area.value;
  setText(text: string) {
    this.area.value = text;
    this.area.setSelectionRange(0, 0);
  }
  setReadOnly = (readOnly: boolean) => (this.area.readOnly = readOnly);
  onChange = (listener: (edited: boolean) => void) => (this.#listener = listener);
  focus = () => this.area.focus();
  openSearch = (replace: boolean) => this.searches.push(replace);
  destroy = () => this.area.remove();

  cursor() {
    const before = this.area.value.slice(0, this.area.selectionStart).split('\n');

    return { line: before.length, col: before[before.length - 1].length + 1 };
  }
}

/** The module `$lib/editor/cm` is replaced with: `vi.mock('$lib/editor/cm', () => import('../tests/editor'))`. */
export const createSurface = (options: SurfaceOptions): EditorSurface => new FakeSurface(options);

/** A text file as the backend opens it. */
export function editDocument(extra: Partial<EditDocument> = {}): EditDocument {
  return {
    name: 'notes.txt',
    text: 'one\ntwo\n',
    encoding: 'UTF-8',
    bom: false,
    lineEnding: 'lf',
    mixedEndings: false,
    readOnly: false,
    size: 8,
    revision: 'r1',
    ...extra,
  };
}

/**
 * The answers of a fake backend for the editor: no external editor is set up, `edit_open` gives `document` (in the
 * encoding it is asked for), `edit_save` saves and moves the revision on. `extra` replaces any of them.
 */
export function editAnswers(document: Partial<EditDocument> = {}, extra: Answers = {}): Answers {
  let saves = 0;

  return {
    edit_external: () => 'notConfigured',
    edit_open: (args: { encoding: string | null }) => ({
      kind: 'text',
      document: editDocument({ ...document, encoding: args.encoding ?? document.encoding ?? 'UTF-8' }),
    }),
    edit_save: () => {
      saves += 1;
      return { kind: 'saved', size: 9, revision: `s${saves}` };
    },
    ...extra,
  };
}

/** What the editor is opened on in {@link openEditor}. */
export const TARGET = { path: '/dir', name: 'notes.txt' };

/** An editor on a fake backend, with its overlay on screen; nothing is opened yet. */
export function startEditor(answers: Answers = {}, document: Partial<EditDocument> = {}, target = TARGET) {
  const backend = fakeBackend(editAnswers(document, answers) as Partial<Record<CommandName, (args: never) => unknown>>);
  const say = vi.fn();
  const view = vi.fn();
  const editor = new Editor(backend.ipc, { under: () => target, say, view });
  render(EditorHost, { editor });

  /** The arguments of every call of a command. */
  const sent = (name: string) => backend.invoke.mock.calls.filter(([command]) => command === name).map(([, args]) => args);

  /** The options of every save. */
  const options = () => sent('edit_save').map((args) => args.options as EditSaveOptions);

  return { backend, editor, say, view, sent, options, open: () => editor.open(target, { builtin: true }) };
}

/** The same, opened on {@link TARGET} and waited for. */
export async function openEditor(answers: Answers = {}, document: Partial<EditDocument> = {}) {
  const world = startEditor(answers, document);
  await world.open();
  await screen.findByRole('textbox', { name: `Text of ${TARGET.name}` });

  return world;
}

/** The text of the editor on screen, and typing into it. */
export const typed = () => screen.getByRole('textbox', { name: `Text of ${TARGET.name}` });
export const type = (text: string) => fireEvent.input(typed(), { target: { value: text } });
