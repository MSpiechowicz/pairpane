import type { Preferences } from '$lib/ipc/bindings/Preferences';

import type { SortColumn } from './order';

/** The columns besides the name that the settings can hide, by the preference that shows each. */
export const SHOWN: Readonly<Record<Exclude<SortColumn, 'name'>, 'showExt' | 'showSize' | 'showModified'>> = {
  ext: 'showExt',
  size: 'showSize',
  modified: 'showModified',
};

/** Whether the pane shows `column`; the name is always shown. */
export function shows(column: SortColumn, preferences: Readonly<Preferences>): boolean {
  return column === 'name' || preferences[SHOWN[column]];
}

/** The icon and the name, which are always there. */
const FIXED = '20px minmax(0, 1fr)';

/**
 * The grid of the header and of every row: the icon, the name and the columns that are shown, in the order the
 * header has them. The narrow layout drops Ext and Modified, whatever the settings say about them.
 */
export function columnTracks(preferences: Readonly<Preferences>): { full: string; narrow: string } {
  const size = shows('size', preferences) ? ' 74px' : '';
  const ext = shows('ext', preferences) ? ' 48px' : '';
  const modified = shows('modified', preferences) ? ' 118px' : '';

  return { full: `${FIXED}${ext}${size}${modified}`, narrow: `${FIXED}${size}` };
}
