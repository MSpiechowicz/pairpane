import type { FileAction } from '$lib/ops/dialogs';

/** The dropdown a pane can show under its header. */
export type Menu = 'drives' | 'hotlist';

/** What the Files menu asks for: an action of the file keys, or the file in the built-in editor (F4 may mean another). */
export type FilesChoice = FileAction | 'editBuiltin';
