import { EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';

import { sideSetup } from './cmSetup';
import type { EditorSurface, SurfaceOptions } from './surface';

/** The surface of the editor, made with CodeMirror. With cmSetup and cmMerge, the only files that name it. */
export function createSurface(options: SurfaceOptions): EditorSurface {
  const side = sideSetup(options);
  const stateFor = (doc: string) => EditorState.create({ doc, extensions: side.extensions() });
  const view = new EditorView({ parent: options.parent, state: stateFor(options.text) });

  return side.bind(view, {
    replace: (text) => view.setState(stateFor(text)),
    destroy: () => view.destroy(),
  });
}
