import type { Entry } from '$lib/ipc/bindings/Entry';

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'] as const;

/** `98 B`, `1.3 KB`, `47 KB`, `5.7 GB`: one decimal below 10, none above (binary units, as file managers show). */
export function formatSize(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }

  const digits = unit > 0 && value < 10 ? 1 : 0;
  return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

const pad = (n: number) => String(n).padStart(2, '0');

/** `2026-11-20 07:58` in local time; empty when unknown. */
export function formatDate(millis: number | null): string {
  if (millis === null) {
    return '';
  }

  const date = new Date(millis);
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/**
 * Characters that can make one name pass for another: controls, the ones that reorder text (bidirectional marks and
 * overrides), and the invisible ones that hide or split it (zero-width space, joiners and the invisible operators,
 * the soft hyphen, the byte order mark, the line and paragraph separators, the deprecated formatting characters and
 * the tag characters). The zero-width joiner is escaped too, so an emoji sequence joined with it shows as its parts
 * and the code: the price of leaving no invisible character in a name.
 */
const MISLEADING =
  /[\p{Cc}\u00AD\u061C\u180E\u200B-\u200F\u2028-\u202E\u2060-\u2064\u2066-\u206F\uFEFF\u{E0000}-\u{E007F}]/gu;

/**
 * A name (or a path) as it is shown: each control, bidirectional-formatting and invisible character is written out as `⟨U+202E⟩`,
 * so that a name cannot read as another (`txt.exe` behind a right-to-left override). Only for display: the real name
 * is what goes to the backend.
 */
export function displayName(name: string): string {
  return name.replace(MISLEADING, (char) => `⟨U+${char.codePointAt(0)!.toString(16).toUpperCase().padStart(4, '0')}⟩`);
}

/** Name and extension as the columns show them. Folders and dot files (`.bashrc`) have no extension. */
export function splitName(entry: Entry): { base: string; ext: string } {
  const dot = entry.name.lastIndexOf('.');
  if (entry.kind === 'dir' || dot <= 0 || dot === entry.name.length - 1) {
    return { base: entry.name, ext: '' };
  }

  return { base: entry.name.slice(0, dot), ext: entry.name.slice(dot + 1) };
}

const ICONS: Record<string, readonly string[]> = {
  img: ['png', 'jpg', 'jpeg', 'gif', 'webp', 'heic', 'svg', 'bmp', 'tiff', 'avif', 'ico'],
  code: ['rs', 'ts', 'js', 'svelte', 'py', 'go', 'c', 'h', 'cpp', 'java', 'kt', 'toml', 'json', 'yml', 'yaml', 'lock', 'sh'],
  arc: ['zip', 'tar', 'gz', 'xz', 'zst', '7z', 'rar', 'bz2', 'iso', 'dmg'],
  doc: ['md', 'txt', 'pdf', 'doc', 'docx', 'odt', 'xlsx', 'csv', 'rtf'],
  exe: ['exe', 'msi', 'appimage', 'deb', 'rpm', 'app'],
};

/** The icon class for an entry: `dir`, a file category, or plain `f`. */
export function iconClass(entry: Entry): string {
  if (entry.kind === 'dir') {
    return 'dir';
  }

  const ext = splitName(entry).ext.toLowerCase();
  return Object.keys(ICONS).find((category) => ICONS[category].includes(ext)) ?? 'f';
}

/** `4 folders · 5 files · 89 KB` */
export function formatSummary(dirs: number, files: number, bytes: number): string {
  const plural = (n: number, word: string) => `${n.toLocaleString('en-US')} ${word}${n === 1 ? '' : 's'}`;

  return `${plural(dirs, 'folder')} · ${plural(files, 'file')} · ${formatSize(bytes)}`;
}

/** `Selected: 1 folder · 2 files · 2.0 KB` */
export function formatSelection(dirs: number, files: number, bytes: number): string {
  return `Selected: ${formatSummary(dirs, files, bytes)}`;
}

/** `5.7 GB free` */
export function formatFree(bytes: number): string {
  return `${formatSize(bytes)} free`;
}
