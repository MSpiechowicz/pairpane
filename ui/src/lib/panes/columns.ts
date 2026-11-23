import type { Preferences } from '$lib/ipc/bindings/Preferences';

import type { SortColumn } from './order';

/** One column of the detailed layout besides the name. */
interface ColumnSpec {
  column: Exclude<SortColumn, 'name'>;
  /** The header's button. */
  label: string;
  /** The class of its cell in the header and in a row (the stylesheets hide the cells of the columns the narrow layout drops). */
  cell: string;
  /** The preference that shows the column. */
  shownBy: Exclude<keyof Preferences, 'theme'>;
  /** The width of its track in the grid. */
  width: string;
  /** The narrow layout keeps the column. */
  narrow: boolean;
}

/**
 * The columns the settings can hide, in the order the header and the rows have them. A column is added here (with its
 * preference), in the cells of a row and in the stylesheets of the header and the row (for the narrow layout).
 */
export const COLUMNS: readonly ColumnSpec[] = [
  { column: 'ext', label: 'Ext', cell: 'ex', shownBy: 'showExt', width: '48px', narrow: false },
  { column: 'size', label: 'Size', cell: 'sz', shownBy: 'showSize', width: '74px', narrow: true },
  { column: 'modified', label: 'Modified', cell: 'dt', shownBy: 'showModified', width: '118px', narrow: false },
  { column: 'dimensions', label: 'Dimensions', cell: 'dm', shownBy: 'showDimensions', width: '92px', narrow: false },
  { column: 'checksum', label: 'Checksum', cell: 'ck', shownBy: 'showChecksum', width: '104px', narrow: false },
  { column: 'git', label: 'Git', cell: 'gt', shownBy: 'showGit', width: '44px', narrow: false },
];

/** Whether the pane shows `column`; the name is always shown. */
export function shows(column: SortColumn, preferences: Readonly<Preferences>): boolean {
  return column === 'name' || COLUMNS.some((spec) => spec.column === column && preferences[spec.shownBy]);
}

/** The icon and the name, which are always there. */
const FIXED = '20px minmax(0, 1fr)';

/**
 * The grid of the header and of every row: the icon, the name and the columns that are shown, in the order the
 * header has them. The narrow layout drops the columns that do not stay in it, whatever the settings say about them.
 */
export function columnTracks(preferences: Readonly<Preferences>): { full: string; narrow: string } {
  const shown = COLUMNS.filter((spec) => preferences[spec.shownBy]);
  const tracks = (specs: readonly ColumnSpec[]) => specs.map((spec) => ` ${spec.width}`).join('');

  return { full: `${FIXED}${tracks(shown)}`, narrow: `${FIXED}${tracks(shown.filter((spec) => spec.narrow))}` };
}
