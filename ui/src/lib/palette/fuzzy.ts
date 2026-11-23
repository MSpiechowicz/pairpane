/** Where a word starts: after one of these, or at the start. */
const SEPARATORS = new Set([' ', '/', '\\', '-', '_', '.', '…']);

/**
 * How well `query` matches `text`, its characters in order (case ignored), higher being better; `undefined` when it does
 * not. A match at the start of a word and characters that follow one another count more, a gap less; an empty query
 * matches everything with 0.
 */
export function score(query: string, text: string): number | undefined {
  const wanted = query.trim().toLowerCase();
  if (wanted === '') {
    return 0;
  }

  const lower = text.toLowerCase();
  let total = 0;
  let at = 0;
  let previous = -2;

  for (const char of wanted) {
    if (char === ' ') {
      continue;
    }

    const found = lower.indexOf(char, at);
    if (found < 0) {
      return undefined;
    }

    const wordStart = found === 0 || SEPARATORS.has(lower[found - 1]) || (text[found] !== lower[found] && text[found - 1] === lower[found - 1]);
    total += 1 + (found === previous + 1 ? 7 : 0) + (wordStart ? 6 : 0) - Math.min(found - at, 15) * 0.5;
    previous = found;
    at = found + 1;
  }

  // Shorter texts with the same match are closer to what was typed.
  return total - text.length * 0.01;
}
