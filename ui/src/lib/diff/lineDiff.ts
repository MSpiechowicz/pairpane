import { matchLines, type LineBlock } from './lineMatch';

/** A changed range: A[fromA, toA) became B[fromB, toB), in characters of the two texts. */
export interface Span {
  fromA: number;
  toA: number;
  fromB: number;
  toB: number;
}

/** A character-level diff of two short texts; the spans it returns are relative to the start of each text. */
export type CharDiff = (a: string, b: string) => readonly Span[];

/**
 * What the character-level refinement may spend. A block is refined only while both its sides (after the common
 * start and end are cut) are at most `side` characters, and every refinement costs `call` plus its size product
 * (counted up to `product`: the diff of two long unrelated texts gives up early, so it costs no more than that)
 * against `cost`; a block past either limit is reported as one span, which still marks its whole lines. A block
 * of as many lines on both sides is refined line against line instead, for at most `pairs` lines per diff, so a
 * long run of edited lines keeps its marks and the number of changes stays bounded. Measured, a unit of cost is
 * 30 ns at most, so a whole diff spends about 120 ms on this in the worst case, unrelated text included.
 */
const REFINE = { side: 2000, cost: 4_000_000, call: 300, product: 250_000, pairs: 50_000 };

/** Two changes closer than this in both texts are one change (the merge view joins them anyway, and fewer objects are cheaper). */
const MIN_GAP = 3;

/**
 * The changes between two texts, for a merge view: a line-level diff (see `matchLines`) finds the changed blocks,
 * which are then refined to the changed characters with `refine`, within a fixed budget. Never does more than a
 * bounded amount of work per line of input plus the budget, whatever the texts hold.
 */
export function diffText(a: string, b: string, refine: CharDiff): Span[] {
  if (a === b) {
    return [];
  }

  const linesA = a.split('\n');
  const linesB = b.split('\n');
  const ids = new Map<string, number>();
  const idsA = toIds(linesA, ids);
  const idsB = toIds(linesB, ids);
  const blocks = matchLines(idsA, idsB, ids.size);
  const work: Work = { a, b, startsA: lineStarts(linesA), startsB: lineStarts(linesB), refine, cost: REFINE.cost, pairs: REFINE.pairs };
  const spans: Span[] = [];

  for (const block of blocks) {
    for (const span of refineBlock(work, block)) {
      push(spans, span);
    }
  }

  return spans;
}

/** The two texts, where their lines start, the refinement and what is left of its budget. */
interface Work {
  a: string;
  b: string;
  startsA: Float64Array;
  startsB: Float64Array;
  refine: CharDiff;
  cost: number;
  pairs: number;
}

function toIds(lines: string[], ids: Map<string, number>): Int32Array {
  const out = new Int32Array(lines.length);

  for (let i = 0; i < lines.length; i++) {
    let id = ids.get(lines[i]);

    if (id === undefined) {
      id = ids.size;
      ids.set(lines[i], id);
    }

    out[i] = id;
  }

  return out;
}

/** The offset of each line, and one more for the end: where a line after the last would start. */
function lineStarts(lines: string[]): Float64Array {
  const starts = new Float64Array(lines.length + 1);

  for (let i = 0; i < lines.length; i++) {
    starts[i + 1] = starts[i] + lines[i].length + 1;
  }

  return starts;
}

/**
 * The characters of a block of lines, line breaks included. A text that does not end in a line break has none after
 * its last line, so a block of added or removed lines there takes the break before it instead, on both sides.
 */
function charRanges(block: LineBlock, work: Work): Span {
  const { startsA, startsB } = work;
  const lenA = work.a.length;
  const lenB = work.b.length;
  let fromA = startsA[block.aFrom];
  let fromB = startsB[block.bFrom];

  if (fromA > lenA) {
    fromA = lenA;
    fromB -= 1;
  } else if (fromB > lenB) {
    fromB = lenB;
    fromA -= 1;
  }

  const toA = block.aTo === block.aFrom ? fromA : Math.min(startsA[block.aTo], lenA);
  const toB = block.bTo === block.bFrom ? fromB : Math.min(startsB[block.bTo], lenB);

  return { fromA, toA, fromB, toB };
}

/** The spans inside one block: the changed characters when the block is small enough to look at and the budget lasts, else one span (or one per line pair). */
function refineBlock(work: Work, block: LineBlock): Span[] {
  const whole = charRanges(block, work);

  if (whole.fromA === whole.toA || whole.fromB === whole.toB) {
    return [whole];
  }

  const refined = refineText(work, whole);

  if (refined !== undefined) {
    return refined;
  }

  const lines = block.aTo - block.aFrom;

  if (lines === 1 || lines !== block.bTo - block.bFrom) {
    return [trim(work.a, work.b, whole)];
  }

  return pairLines(work, block, whole);
}

/** The changed characters of a pair of texts, or undefined when they are too big for what is left of the budget. */
function refineText(work: Work, range: Span): Span[] | undefined {
  const trimmed = trim(work.a, work.b, range);
  const lenA = trimmed.toA - trimmed.fromA;
  const lenB = trimmed.toB - trimmed.fromB;

  if (lenA === 0 || lenB === 0 || (lenA === 1 && lenB === 1)) {
    return [trimmed];
  }

  const cost = Math.min(lenA * lenB, REFINE.product) + REFINE.call;

  if (lenA > REFINE.side || lenB > REFINE.side || cost > work.cost) {
    return undefined;
  }

  work.cost -= cost;

  return work.refine(work.a.slice(trimmed.fromA, trimmed.toA), work.b.slice(trimmed.fromB, trimmed.toB)).map((span) => ({
    fromA: span.fromA + trimmed.fromA,
    toA: span.toA + trimmed.fromA,
    fromB: span.fromB + trimmed.fromB,
    toB: span.toB + trimmed.fromB,
  }));
}

/**
 * A block with as many lines on both sides, refined line against line, for as many lines as the budget allows; the
 * rest of the block is one span.
 */
function pairLines(work: Work, block: LineBlock, whole: Span): Span[] {
  const { a, b, startsA, startsB } = work;
  const spans: Span[] = [];

  for (let i = 0; i < block.aTo - block.aFrom; i++) {
    const fromA = startsA[block.aFrom + i];
    const fromB = startsB[block.bFrom + i];
    const line = {
      fromA,
      toA: Math.min(startsA[block.aFrom + i + 1] - 1, a.length),
      fromB,
      toB: Math.min(startsB[block.bFrom + i + 1] - 1, b.length),
    };

    if (a.slice(line.fromA, line.toA) === b.slice(line.fromB, line.toB)) {
      continue;
    }

    if (work.pairs-- <= 0) {
      spans.push(trim(a, b, { fromA, toA: whole.toA, fromB, toB: whole.toB }));
      break;
    }

    for (const span of refineText(work, line) ?? [trim(a, b, line)]) {
      spans.push(span);
    }
  }

  return spans;
}

/** The span without the characters both sides share at its start and its end (never cutting a surrogate pair in two). */
function trim(a: string, b: string, span: Span): Span {
  let { fromA, toA, fromB, toB } = span;

  while (fromA < toA && fromB < toB && a.charCodeAt(fromA) === b.charCodeAt(fromB)) {
    fromA++;
    fromB++;
  }

  while (fromA < toA && fromB < toB && a.charCodeAt(toA - 1) === b.charCodeAt(toB - 1)) {
    toA--;
    toB--;
  }

  if (fromA > span.fromA && (isLowSurrogate(a, fromA) || isLowSurrogate(b, fromB))) {
    fromA--;
    fromB--;
  }

  if (toA < span.toA && isLowSurrogate(a, toA)) {
    toA++;
    toB++;
  }

  return { fromA, toA, fromB, toB };
}

function isLowSurrogate(text: string, at: number): boolean {
  const code = text.charCodeAt(at);

  return code >= 0xdc00 && code <= 0xdfff;
}

/** Adds a span, joining it to the one before when they are closer than `MIN_GAP`. */
function push(spans: Span[], span: Span): void {
  const last = spans[spans.length - 1];

  if (last !== undefined && last.toA > span.fromA - MIN_GAP && last.toB > span.fromB - MIN_GAP) {
    last.toA = span.toA;
    last.toB = span.toB;
  } else {
    spans.push(span);
  }
}
