import type { RenameProblem } from '$lib/ipc/bindings/RenameProblem';

/** What the preview says of a row that cannot be renamed. */
export const PROBLEMS: Record<RenameProblem, string> = {
  empty: 'The new name is empty.',
  invalid: 'Not a valid file name.',
  tooLong: 'The new name is too long.',
  duplicate: 'Another item gets the same name.',
  exists: 'An item with this name is already there.',
  missing: 'The item is gone.',
  unreadable: 'The name is not valid text.',
  noDate: 'The item has no modification date.',
};
