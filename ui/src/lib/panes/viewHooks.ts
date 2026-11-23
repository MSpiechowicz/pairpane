import type { Entry } from '$lib/ipc/bindings/Entry';

import type { ExtraColumn } from './order';

/** What the owner of a pane (its tabs) needs to hear about its folders, and gives it. */
export interface ViewHooks {
  /** An open begins (any open, a silent one included), before any of its events. */
  onOpen?: () => void;
  /** A folder started listing, the shown one again included: where the pane is now. */
  onListed?: (path: string) => void;
  /** An open failed and the pane has nothing to show. */
  onFail?: () => void;
  /** An open of another folder failed and the pane stays in the folder it had. */
  onNotice?: () => void;
  /** Whether hidden entries are listed (the preference). */
  showHidden?: () => boolean;
  /** What a pane sorted by an extra column has to go by: the value of the column for an entry, once it is known. */
  extraOf?: (entry: Entry, column: ExtraColumn) => number | string | undefined;
}
