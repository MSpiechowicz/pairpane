/** Where a word starts: after one of these, or at the start. */
const SEPARATORS = new Set([' ', '/', '\\', '-', '_', '.', '…']);

/** How many places of the first letter a match is tried from. */
const STARTS = 8;

/**
 * How well `query` matches `text`, its characters in order (case and the form of accented letters ignored), higher being
 * better; `undefined` when it does not. A match at the start of a word and characters that follow one another count more,
 * a gap less; the match is tried from a few places of the first letter and the best is kept (`tab` is the word of
 * "Next tab", not its `t`). An empty query matches everything with 0.
 */
export function score(query: string, text: string): number | undefined {
  const wanted = [...query.normalize('NFC').toLowerCase()].filter((char) => char.trim() !== '');
  if (wanted.length === 0) {
    return 0;
  }

  const chars = [...text.normalize('NFC')];
  const lower = chars.map((char) => char.toLowerCase());
  let best: number | undefined;
  let start = lower.indexOf(wanted[0]);

  for (let tries = 0; start >= 0 && tries < STARTS; tries += 1) {
    const found = from(wanted, chars, lower, start);
    if (found === undefined) {
      break;
    }
    best = best === undefined ? found : Math.max(best, found);
    start = lower.indexOf(wanted[0], start + 1);
  }

  // Shorter texts with the same match are closer to what was typed.
  return best === undefined ? undefined : best - chars.length * 0.01;
}

/** The score of the match of `wanted` in the characters `chars` (`lower` in lower case) that starts at `start`. */
function from(wanted: readonly string[], chars: readonly string[], lower: readonly string[], start: number): number | undefined {
  let total = 0;
  let at = 0;
  let previous = -2;

  for (const [index, char] of wanted.entries()) {
    const found = index === 0 ? start : lower.indexOf(char, at);
    if (found < 0) {
      return undefined;
    }

    const upper = chars[found] !== lower[found] && chars[found - 1] === lower[found - 1];
    const wordStart = found === 0 || SEPARATORS.has(lower[found - 1]) || upper;
    total += 1 + (found === previous + 1 ? 7 : 0) + (wordStart ? 6 : 0) - Math.min(found - at, 15) * 0.5;
    previous = found;
    at = found + 1;
  }

  return total;
}
