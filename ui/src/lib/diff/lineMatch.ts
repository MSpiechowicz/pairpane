/** A changed stretch of lines: A[aFrom, aTo) was replaced by B[bFrom, bTo). One side may be empty (a pure insertion or deletion). */
export interface LineBlock {
  aFrom: number;
  aTo: number;
  bFrom: number;
  bTo: number;
}

/**
 * The most edits a stretch with no line unique to both sides is searched for (the search keeps one frame per edit, so
 * its memory and its time grow with the square of this). A stretch needing more is reported as one replaced block.
 */
const MAX_EDITS = 400;

/** The total work (line visits) allowed per line of input, plus a constant: past it the rest is reported as replaced blocks. */
const WORK_PER_LINE = 12;
const WORK_BASE = 1_000_000;

/**
 * The changed blocks between two sequences of line ids (equal lines share an id, ids are below `idCount`), in order.
 *
 * Common lines at the ends are dropped; a stretch is then split at the lines that occur exactly once on each side
 * (the longest run of them that keeps their order: patience diff) and every gap is handled the same way. A gap
 * with no such line goes to a bounded Myers search. The work has a hard total bound, linear in the input: what it
 * cannot reach is returned as one replaced block, which is correct, only coarser.
 */
export function matchLines(a: ArrayLike<number>, b: ArrayLike<number>, idCount: number): LineBlock[] {
  const blocks: LineBlock[] = [];
  const emit = (aFrom: number, aTo: number, bFrom: number, bTo: number) => {
    const last = blocks[blocks.length - 1];

    if (last !== undefined && last.aTo === aFrom && last.bTo === bFrom) {
      last.aTo = aTo;
      last.bTo = bTo;
    } else {
      blocks.push({ aFrom, aTo, bFrom, bTo });
    }
  };

  const countA = new Int32Array(idCount);
  const countB = new Int32Array(idCount);
  const where = new Int32Array(idCount);
  const budget = { work: WORK_PER_LINE * (a.length + b.length) + WORK_BASE };

  // Ranges wait here, the leftmost on top, so the blocks come out in order and a deep split cannot overflow the call stack.
  const stack: number[] = [0, a.length, 0, b.length];

  while (stack.length > 0) {
    let bTo = stack.pop() as number;
    let bFrom = stack.pop() as number;
    let aTo = stack.pop() as number;
    let aFrom = stack.pop() as number;

    while (aFrom < aTo && bFrom < bTo && a[aFrom] === b[bFrom]) {
      aFrom++;
      bFrom++;
    }

    while (aFrom < aTo && bFrom < bTo && a[aTo - 1] === b[bTo - 1]) {
      aTo--;
      bTo--;
    }

    if (aFrom === aTo && bFrom === bTo) {
      continue;
    }

    budget.work -= aTo - aFrom + (bTo - bFrom);

    if (aFrom === aTo || bFrom === bTo || budget.work < 0) {
      emit(aFrom, aTo, bFrom, bTo);
      continue;
    }

    const anchors = uniqueAnchors(a, b, [aFrom, aTo, bFrom, bTo], countA, countB, where);

    if (anchors.length === 0) {
      const found = myers(a, b, [aFrom, aTo, bFrom, bTo], budget);

      if (found === undefined) {
        emit(aFrom, aTo, bFrom, bTo);
      } else {
        for (const block of found) {
          emit(block.aFrom, block.aTo, block.bFrom, block.bTo);
        }
      }

      continue;
    }

    // The gaps between the anchors, pushed last to first.
    let nextA = aTo;
    let nextB = bTo;

    for (let i = anchors.length - 1; i >= 0; i--) {
      const [anchorA, anchorB] = anchors[i];
      stack.push(anchorA + 1, nextA, anchorB + 1, nextB);
      nextA = anchorA;
      nextB = anchorB;
    }

    stack.push(aFrom, nextA, bFrom, nextB);
  }

  return blocks;
}

/** The lines that occur once in each of the two ranges, as the longest run of them that is in the same order on both sides. */
function uniqueAnchors(
  a: ArrayLike<number>,
  b: ArrayLike<number>,
  [aFrom, aTo, bFrom, bTo]: [number, number, number, number],
  countA: Int32Array,
  countB: Int32Array,
  where: Int32Array,
): Array<[number, number]> {
  for (let i = aFrom; i < aTo; i++) {
    countA[a[i]]++;
  }

  for (let j = bFrom; j < bTo; j++) {
    countB[b[j]]++;
    where[b[j]] = j;
  }

  const pairs: Array<[number, number]> = [];

  for (let i = aFrom; i < aTo; i++) {
    const id = a[i];

    if (countA[id] === 1 && countB[id] === 1) {
      pairs.push([i, where[id]]);
    }
  }

  // The counts go back to zero for the next range.
  for (let i = aFrom; i < aTo; i++) {
    countA[a[i]] = 0;
  }

  for (let j = bFrom; j < bTo; j++) {
    countB[b[j]] = 0;
  }

  return longestIncreasing(pairs);
}

/** The longest run of pairs whose second numbers grow, taking the pairs (already ordered by their first) as they come: patience sorting. */
function longestIncreasing(pairs: Array<[number, number]>): Array<[number, number]> {
  const tails: number[] = [];
  const previous = new Int32Array(pairs.length).fill(-1);

  for (let k = 0; k < pairs.length; k++) {
    let lo = 0;
    let hi = tails.length;

    while (lo < hi) {
      const mid = (lo + hi) >> 1;

      if (pairs[tails[mid]][1] < pairs[k][1]) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }

    previous[k] = lo > 0 ? tails[lo - 1] : -1;
    tails[lo] = k;
  }

  const run: Array<[number, number]> = [];

  for (let k = tails.length > 0 ? tails[tails.length - 1] : -1; k >= 0; k = previous[k]) {
    run.push(pairs[k]);
  }

  return run.reverse();
}

/**
 * The shortest edit script between two ranges by Myers' O(ND) search, or undefined when it needs more than
 * `MAX_EDITS` edits or more than the remaining work.
 */
function myers(
  a: ArrayLike<number>,
  b: ArrayLike<number>,
  [aFrom, aTo, bFrom, bTo]: [number, number, number, number],
  budget: { work: number },
): LineBlock[] | undefined {
  const n = aTo - aFrom;
  const m = bTo - bFrom;
  const limit = Math.min(n + m, MAX_EDITS);
  const offset = limit + 1;
  const reach = new Int32Array(2 * limit + 3);
  const frames: Int32Array[] = [];

  for (let d = 0; d <= limit; d++) {
    for (let k = -d; k <= d; k += 2) {
      let x: number;

      if (k === -d || (k !== d && reach[offset + k - 1] < reach[offset + k + 1])) {
        x = reach[offset + k + 1];
      } else {
        x = reach[offset + k - 1] + 1;
      }

      let y = x - k;

      while (x < n && y < m && a[aFrom + x] === b[bFrom + y]) {
        x++;
        y++;
        budget.work--;
      }

      reach[offset + k] = x;

      if (x === n && y === m) {
        frames.push(reach.slice(offset - d, offset + d + 1));

        return script(frames, [n, m], [aFrom, bFrom]);
      }
    }

    budget.work -= d + 1;
    frames.push(reach.slice(offset - d, offset + d + 1));

    if (budget.work < 0) {
      return undefined;
    }
  }

  return undefined;
}

/** Walks the frames of a finished Myers search back from the corner and returns the edits as blocks, in order. */
function script(frames: Int32Array[], [n, m]: [number, number], [aFrom, bFrom]: [number, number]): LineBlock[] {
  const edits: LineBlock[] = [];
  let x = n;
  let y = m;

  for (let d = frames.length - 1; d > 0; d--) {
    const previous = frames[d - 1];
    const k = x - y;
    const at = (diagonal: number) => previous[diagonal + (d - 1)];
    const inserted = k === -d || (k !== d && at(k - 1) < at(k + 1));
    const fromX = inserted ? at(k + 1) : at(k - 1);
    const fromY = fromX - (inserted ? k + 1 : k - 1);

    if (inserted) {
      edits.push({ aFrom: aFrom + fromX, aTo: aFrom + fromX, bFrom: bFrom + fromY, bTo: bFrom + fromY + 1 });
    } else {
      edits.push({ aFrom: aFrom + fromX, aTo: aFrom + fromX + 1, bFrom: bFrom + fromY, bTo: bFrom + fromY });
    }

    x = fromX;
    y = fromY;
  }

  const blocks: LineBlock[] = [];

  for (let i = edits.length - 1; i >= 0; i--) {
    const edit = edits[i];
    const last = blocks[blocks.length - 1];

    if (last !== undefined && last.aTo === edit.aFrom && last.bTo === edit.bFrom) {
      last.aTo = edit.aTo;
      last.bTo = edit.bTo;
    } else {
      blocks.push(edit);
    }
  }

  return blocks;
}
