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
