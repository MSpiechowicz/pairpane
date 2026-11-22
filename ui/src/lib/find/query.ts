import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';
import type { Tri } from '$lib/ipc/bindings/Tri';

/** The units a size is typed in (binary, as the panes show sizes). */
export const SIZE_UNITS = [
  { label: 'B', bytes: 1 },
  { label: 'KB', bytes: 1024 },
  { label: 'MB', bytes: 1024 ** 2 },
  { label: 'GB', bytes: 1024 ** 3 },
] as const;

export type SizeUnit = (typeof SIZE_UNITS)[number]['label'];

/** What the dialog's fields hold: text as typed, dates as `YYYY-MM-DD` of the local calendar, a blank for "not set". */
export interface FindForm {
  names: string;
  nameRegex: boolean;
  namePath: boolean;
  nameCase: boolean;
  text: string;
  textRegex: boolean;
  textCase: boolean;
  wholeWord: boolean;
  /** An encoding the viewer offers; blank reads UTF-8 and the byte order marks. */
  encoding: string;
  anySize: boolean;
  minSize: string;
  minUnit: SizeUnit;
  maxSize: string;
  maxUnit: SizeUnit;
  from: string;
  to: string;
  hidden: Tri;
  readonly: Tri;
  executable: Tri;
  skipHidden: boolean;
  skipIgnored: boolean;
  oneFilesystem: boolean;
  archives: boolean;
  duplicates: boolean;
}

export function blankForm(): FindForm {
  return {
    names: '',
    nameRegex: false,
    namePath: false,
    nameCase: false,
    text: '',
    textRegex: false,
    textCase: false,
    wholeWord: false,
    encoding: '',
    anySize: false,
    minSize: '',
    minUnit: 'KB',
    maxSize: '',
    maxUnit: 'KB',
    from: '',
    to: '',
    hidden: 'any',
    readonly: 'any',
    executable: 'any',
    skipHidden: false,
    skipIgnored: false,
    oneFilesystem: false,
    archives: false,
    duplicates: false,
  };
}

/** Which fields the backend would refuse the current choices of, so the dialog can switch them off. */
export interface Blocked {
  /** The text fields: archives are searched by name only. */
  text: boolean;
  archives: boolean;
  duplicates: boolean;
  /** "Skip hidden" and "hidden: yes" cannot both hold. */
  skipHidden: boolean;
  hiddenYes: boolean;
}

export function blocked(form: FindForm): Blocked {
  return {
    text: form.archives,
    archives: form.duplicates || form.text !== '',
    duplicates: form.archives,
    skipHidden: form.hidden === 'yes',
    hiddenYes: form.skipHidden,
  };
}

/** The bytes of a typed size: `null` for a blank, `undefined` for something that is no size. */
function parseSize(amount: string, unit: SizeUnit): number | null | undefined {
  const text = amount.trim();
  if (text === '') {
    return null;
  }

  const value = Number(text);
  const bytes = Math.round(value * (SIZE_UNITS.find((candidate) => candidate.label === unit)?.bytes ?? 1));

  return Number.isFinite(value) && value >= 0 && Number.isSafeInteger(bytes) ? bytes : undefined;
}

/** The size as a number and the biggest unit that holds it in whole numbers. */
function splitSize(bytes: number | null): { amount: string; unit: SizeUnit } {
  if (bytes === null) {
    return { amount: '', unit: 'KB' };
  }

  const unit = [...SIZE_UNITS].reverse().find((candidate) => bytes >= candidate.bytes && bytes % candidate.bytes === 0);
  return unit === undefined ? { amount: String(bytes), unit: 'B' } : { amount: String(bytes / unit.bytes), unit: unit.label };
}

/** The first moment of the local day `YYYY-MM-DD` (or just after its last one: `next`); `undefined` for no such day. */
function localDay(text: string, next: boolean): number | undefined {
  const parts = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text.trim());
  if (parts === null) {
    return undefined;
  }

  const [year, month, day] = [Number(parts[1]), Number(parts[2]), Number(parts[3])];
  const date = new Date(0);
  date.setFullYear(year, month - 1, day);
  date.setHours(0, 0, 0, 0);
  if (date.getFullYear() !== year || date.getMonth() !== month - 1 || date.getDate() !== day) {
    return undefined;
  }

  if (next) {
    date.setDate(date.getDate() + 1);
  }

  return date.getTime();
}

const pad = (n: number, width = 2) => String(n).padStart(width, '0');

/** The local day of a moment, as `YYYY-MM-DD`. */
function dayOf(millis: number | null): string {
  if (millis === null) {
    return '';
  }

  const date = new Date(millis);
  return `${pad(date.getFullYear(), 4)}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** Why the form cannot be searched, or its criteria. `windows`: the executable bit is not a thing there. */
export function toCriteria(form: FindForm, windows: boolean): FindCriteria | string {
  const minSize = parseSize(form.minSize, form.minUnit);
  const maxSize = parseSize(form.maxSize, form.maxUnit);
  if (minSize === undefined || maxSize === undefined) {
    return 'Type a size as a number, like 10 or 1.5.';
  }

  if (minSize !== null && maxSize !== null && minSize > maxSize) {
    return 'The smallest size is above the largest.';
  }

  const from = form.from.trim() === '' ? null : localDay(form.from, false);
  const to = form.to.trim() === '' ? null : localDay(form.to, true);
  if (from === undefined || to === undefined) {
    return 'Type a date as year-month-day, like 2026-11-20.';
  }

  // The last moment of the day: "to" includes the whole day.
  const modifiedTo = to === null ? null : to - 1;
  if (from !== null && modifiedTo !== null && from > modifiedTo) {
    return 'The first date is after the last.';
  }

  const refused = incompatible(form);
  if (refused !== undefined) {
    return refused;
  }

  return {
    names: form.names.trim(),
    nameRegex: form.nameRegex,
    namePath: form.namePath,
    nameCase: form.nameCase,
    text:
      form.text === ''
        ? null
        : {
            pattern: form.text,
            regex: form.textRegex,
            caseSensitive: form.textCase,
            wholeWord: form.wholeWord,
            encoding: form.encoding === '' ? null : form.encoding,
            anySize: form.anySize,
          },
    minSize,
    maxSize,
    modifiedFrom: from,
    modifiedTo,
    hidden: form.hidden,
    readonly: form.readonly,
    executable: windows ? 'any' : form.executable,
    skipHidden: form.skipHidden,
    skipIgnored: form.skipIgnored,
    oneFilesystem: form.oneFilesystem,
    archives: form.archives,
    duplicates: form.duplicates,
  };
}

/** What the backend refuses of a combination of choices the fields let through (a loaded search, a typed text). */
function incompatible(form: FindForm): string | undefined {
  if (form.archives && form.duplicates) {
    return 'Archives are searched by name only: not for duplicates.';
  }

  if (form.archives && form.text !== '') {
    return 'Archives are searched by name only: not for text.';
  }

  if (form.skipHidden && form.hidden === 'yes') {
    return 'Hidden files are skipped and asked for at once.';
  }

  if (form.text.includes('\n') || form.text.includes('\r')) {
    return 'The text must be on one line.';
  }

  return undefined;
}

/** The form that holds `criteria`, for a saved search that is loaded. */
export function toForm(criteria: FindCriteria): FindForm {
  const min = splitSize(criteria.minSize);
  const max = splitSize(criteria.maxSize);
  const { text } = criteria;

  return {
    names: criteria.names,
    nameRegex: criteria.nameRegex,
    namePath: criteria.namePath,
    nameCase: criteria.nameCase,
    text: text?.pattern ?? '',
    textRegex: text?.regex ?? false,
    textCase: text?.caseSensitive ?? false,
    wholeWord: text?.wholeWord ?? false,
    encoding: text?.encoding ?? '',
    anySize: text?.anySize ?? false,
    minSize: min.amount,
    minUnit: min.unit,
    maxSize: max.amount,
    maxUnit: max.unit,
    from: dayOf(criteria.modifiedFrom),
    to: dayOf(criteria.modifiedTo),
    hidden: criteria.hidden,
    readonly: criteria.readonly,
    executable: criteria.executable,
    skipHidden: criteria.skipHidden,
    skipIgnored: criteria.skipIgnored,
    oneFilesystem: criteria.oneFilesystem,
    archives: criteria.archives,
    duplicates: criteria.duplicates,
  };
}
