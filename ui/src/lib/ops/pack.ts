import type { PackFormat } from '$lib/ipc/bindings/PackFormat';
import type { PackLevel } from '$lib/ipc/bindings/PackLevel';
import { joinPath } from '$lib/panes/archive';

import { folderName } from './jobs';

/** The archives that can be made: the name in the list and the extension the backend requires of the file. */
export const PACK_FORMATS: readonly { format: PackFormat; label: string; extension: string }[] = [
  { format: 'zip', label: 'zip', extension: '.zip' },
  { format: 'tarZst', label: 'tar.zst', extension: '.tar.zst' },
  { format: 'sevenZ', label: '7z', extension: '.7z' },
];

/** The levels in the order the list shows them. */
const LEVELS: readonly { level: PackLevel; label: string }[] = [
  { level: 'store', label: 'Store' },
  { level: 'fast', label: 'Fast' },
  { level: 'normal', label: 'Normal' },
  { level: 'maximum', label: 'Maximum' },
];

/** The levels a format has: a tar.zst is one stream, so there is nothing to store without compressing. */
export function levelsFor(format: PackFormat): readonly { level: PackLevel; label: string }[] {
  return format === 'tarZst' ? LEVELS.filter(({ level }) => level !== 'store') : LEVELS;
}

/** `level` when the format has it, else Normal. */
export function levelOr(format: PackFormat, level: PackLevel): PackLevel {
  return levelsFor(format).some((one) => one.level === level) ? level : 'normal';
}

/** Whether the format can seal an archive with a password (a tar.zst has no way to). */
export const canEncrypt = (format: PackFormat) => format !== 'tarZst';

/** What the dialog remembers for the rest of the session; a password never is. */
export interface PackChoice {
  format: PackFormat;
  level: PackLevel;
  encryptNames: boolean;
}

export const PACK_DEFAULTS: PackChoice = { format: 'zip', level: 'normal', encryptNames: true };

let remembered: PackChoice = PACK_DEFAULTS;

/** The last choice made in this session (the defaults before one), with a level its format has. */
export function rememberedPack(): PackChoice {
  return { ...remembered, level: levelOr(remembered.format, remembered.level) };
}

/** Keeps `choice` for the next dialog. */
export function rememberPack(choice: PackChoice) {
  remembered = { ...choice };
}

const extensionOf = (format: PackFormat) => PACK_FORMATS.find((one) => one.format === format)?.extension ?? '';

/**
 * The archive a pack starts with: in the folder `folder`, named like the item for one item, like the folder the
 * items are in for several (`archive` at a root), with the format's extension.
 */
export function packPath(folder: string, names: readonly string[], source: string, format: PackFormat): string {
  const base = names.length === 1 ? names[0] : (folderName(source) ?? 'archive');
  return joinPath(folder, base + extensionOf(format));
}

/** `path` with the extension of `format`, when it ends with the extension of a format (the longest, `.tar.zst`, included). */
export function swapExtension(path: string, format: PackFormat): string {
  const lower = path.toLowerCase();
  const known = PACK_FORMATS.find(({ extension }) => lower.endsWith(extension));
  return known === undefined ? path : path.slice(0, path.length - known.extension.length) + extensionOf(format);
}
