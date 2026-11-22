import type { HashAlgorithm } from '$lib/ipc/bindings/HashAlgorithm';

/** The checksums to choose from; the file extension is the algorithm's name, which is how Verify finds it again. */
export const HASHES: readonly { algorithm: HashAlgorithm; label: string }[] = [
  { algorithm: 'sha256', label: 'SHA-256' },
  { algorithm: 'blake3', label: 'BLAKE3' },
  { algorithm: 'md5', label: 'MD5' },
];

/** The last folder of a path (`/home/ann/docs` → `docs`); `undefined` at a root (`/`, `C:\`). */
export function folderName(path: string): string | undefined {
  const last = path.split(/[\\/]/).filter((part) => part !== '').at(-1);
  return last === undefined || last.endsWith(':') ? undefined : last;
}

/**
 * The name a checksum file starts with: the item's own name for one item, the folder's for several (`CHECKSUM` at a
 * root), and the algorithm's extension.
 */
export function checksumName(names: readonly string[], folder: string, algorithm: HashAlgorithm): string {
  const base = names.length === 1 ? names[0] : (folderName(folder) ?? 'CHECKSUM');
  return `${base}.${algorithm}`;
}

/** How big the parts of a split file can be, in bytes; the backend refuses less than 1 KB. */
export const MIN_PART = 1024;
/** The backend ends a split that would make more parts than this. */
export const MAX_PARTS = 999;

/** The sizes people split files by: a floppy disk, a Zip disk, a CD, the largest file FAT32 holds. */
export const PART_PRESETS: readonly { label: string; bytes: number }[] = [
  { label: 'Floppy disk (1,457,664 bytes)', bytes: 1_457_664 },
  { label: 'Zip disk (100 MB)', bytes: 104_857_600 },
  { label: 'CD (700 MB)', bytes: 734_003_200 },
  { label: 'FAT32 (4 GB − 1 byte)', bytes: 4_294_967_295 },
];

export const PART_UNITS = [
  { label: 'KB', bytes: 1024 },
  { label: 'MB', bytes: 1024 ** 2 },
  { label: 'GB', bytes: 1024 ** 3 },
] as const;

/** The bytes of `amount` units; `undefined` when it is not a whole number of bytes the backend could take. */
export function customPart(amount: number, unitBytes: number): number | undefined {
  const bytes = Math.round(amount * unitBytes);
  return Number.isSafeInteger(bytes) && bytes >= MIN_PART ? bytes : undefined;
}

/** How many parts a file of `size` bytes makes. */
export const partCount = (size: number, part: number) => Math.max(1, Math.ceil(size / part));

/** What a permission bit or the Windows read-only attribute does: leave it alone, set it, or clear it. */
export type Tri = 'keep' | 'on' | 'off';

/** The permission bits of the Attributes dialog: a row per class of user, a column for read, write, execute. */
export const MODE_ROWS = [
  { label: 'Owner', bits: [0o400, 0o200, 0o100] },
  { label: 'Group', bits: [0o040, 0o020, 0o010] },
  { label: 'Others', bits: [0o004, 0o002, 0o001] },
] as const;

export const MODE_COLUMNS = ['Read', 'Write', 'Execute'] as const;

export const MODE_SPECIAL = [
  { label: 'Set user ID', bit: 0o4000 },
  { label: 'Set group ID', bit: 0o2000 },
  { label: 'Sticky', bit: 0o1000 },
] as const;

/** Every bit of the dialog. */
export const MODE_BITS: readonly number[] = [...MODE_ROWS.flatMap((row) => row.bits), ...MODE_SPECIAL.map((s) => s.bit)];

/** What every item agrees on (set in all, clear in all), else left alone. */
export function agreed(values: readonly boolean[]): Tri {
  if (values.every(Boolean)) {
    return 'on';
  }

  return values.some(Boolean) ? 'keep' : 'off';
}

/** The state each bit starts in: what the items have in common. */
export function startModes(modes: readonly number[]): Record<number, Tri> {
  return Object.fromEntries(MODE_BITS.map((bit) => [bit, agreed(modes.map((mode) => (mode & bit) !== 0))]));
}

/**
 * The bits to add and to remove for the states the user chose. Only what differs from the start is sent, unless
 * `all` is set (a change that goes into folders: what is inside may differ from the folder itself).
 */
export function modeMasks(start: Record<number, Tri>, now: Record<number, Tri>, all: boolean): { add: number; remove: number } {
  let add = 0;
  let remove = 0;

  for (const bit of MODE_BITS) {
    if (now[bit] === 'keep' || (!all && now[bit] === start[bit])) {
      continue;
    }

    if (now[bit] === 'on') {
      add |= bit;
    } else {
      remove |= bit;
    }
  }

  return { add, remove };
}

/** `rwxr-xr-x`, with `s`/`S` and `t`/`T` for the special bits as `ls` shows them. */
function symbolic(mode: number): string {
  const classes = [
    [0o400, 0o200, 0o100, 0o4000, 's'],
    [0o040, 0o020, 0o010, 0o2000, 's'],
    [0o004, 0o002, 0o001, 0o1000, 't'],
  ] as const;

  return classes
    .map(([read, write, execute, special, letter]) => {
      const run = (mode & execute) !== 0;
      const last = (mode & special) === 0 ? (run ? 'x' : '-') : run ? letter : letter.toUpperCase();
      return `${mode & read ? 'r' : '-'}${mode & write ? 'w' : '-'}${last}`;
    })
    .join('');
}

/** `0755 rwxr-xr-x` */
export function formatMode(mode: number): string {
  return `${(mode & 0o7777).toString(8).padStart(4, '0')} ${symbolic(mode)}`;
}

const pad = (n: number) => String(n).padStart(2, '0');

/** `2026-11-20 07:58:12` in local time. */
export function formatStamp(millis: number): string {
  const date = new Date(millis);
  const day = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  return `${day} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

const STAMP = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})(?::(\d{2}))?$/;

/** Milliseconds of a time typed as `2026-11-20 07:58:12` (seconds optional, local time); `undefined` when it is none. */
export function parseStamp(text: string): number | undefined {
  const match = STAMP.exec(text.trim());
  if (match === null) {
    return undefined;
  }

  const [year, month, day, hour, minute, second] = match.slice(1).map((part) => Number(part ?? 0));
  const date = new Date(year, month - 1, day, hour, minute, second);

  // A date that rolled over (month 13, 31 February) is not the one typed.
  const same =
    [date.getFullYear(), date.getMonth() + 1, date.getDate(), date.getHours(), date.getMinutes(), date.getSeconds()].join() ===
    [year, month, day, hour, minute, second].join();
  return same ? date.getTime() : undefined;
}
