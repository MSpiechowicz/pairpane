import type { Entry } from '$lib/ipc/bindings/Entry';

/** Whether `name` fits `pattern` (both already lower-case): `*` is any run of characters, `?` exactly one. */
function fits(pattern: string, name: string): boolean {
  let p = 0;
  let n = 0;
  let star = -1;
  let resume = 0;

  while (n < name.length) {
    if (p < pattern.length && (pattern[p] === '?' || pattern[p] === name[n])) {
      p += 1;
      n += 1;
    } else if (p < pattern.length && pattern[p] === '*') {
      star = p;
      resume = n;
      p += 1;
    } else if (star >= 0) {
      // Let the last `*` swallow one more character and retry; earlier stars never need revisiting.
      resume += 1;
      n = resume;
      p = star + 1;
    } else {
      return false;
    }
  }

  while (p < pattern.length && pattern[p] === '*') {
    p += 1;
  }

  return p === pattern.length;
}

/**
 * A name matcher for a `;`-separated list of wildcard masks, case-insensitive. Empty parts are ignored and an
 * empty mask matches nothing. `*.*` matches every name, with or without a dot. Not a regex: other characters
 * are literal.
 */
export function parseMask(text: string): (name: string) => boolean {
  const masks = text
    .split(';')
    .map((part) => part.trim().toLowerCase())
    .filter((part) => part !== '');

  if (masks.includes('*.*')) {
    return () => true;
  }

  return (name) => {
    const lower = name.toLowerCase();
    return masks.some((mask) => fits(mask, lower));
  };
}

export type MaskMode = 'select' | 'deselect';

/** The names of the files (not folders) among the entries `each` visits that the mask `text` matches. */
export function maskedFiles(each: (visit: (entry: Entry) => void) => void, text: string): string[] {
  const matches = parseMask(text);
  const names: string[] = [];
  each((entry) => {
    if (entry.kind !== 'dir' && matches(entry.name)) {
      names.push(entry.name);
    }
  });

  return names;
}
