import { describe, expect, it } from 'vitest';

import { diffText, type CharDiff, type Span } from './lineDiff';

/** A character diff that reports what lies between the shared start and end of two short texts. */
const trimmedDiff: CharDiff = (a, b) => {
  let start = 0;

  while (start < a.length && start < b.length && a[start] === b[start]) {
    start++;
  }

  let end = 0;

  while (end < a.length - start && end < b.length - start && a[a.length - 1 - end] === b[b.length - 1 - end]) {
    end++;
  }

  return [{ fromA: start, toA: a.length - end, fromB: start, toB: b.length - end }];
};

const diff = (a: string, b: string) => diffText(a, b, trimmedDiff);

/** What the spans say: what lies outside them is the same in both texts, and B is A with each span's text swapped for B's. */
function expectFaithful(a: string, b: string, spans: readonly Span[]) {
  let posA = 0;
  let posB = 0;
  let rebuilt = '';

  for (const span of spans) {
    expect(span.fromA).toBeGreaterThanOrEqual(posA);
    expect(span.fromB).toBeGreaterThanOrEqual(posB);
    expect(a.slice(posA, span.fromA)).toBe(b.slice(posB, span.fromB));
    expect(span.toA - span.fromA + span.toB - span.fromB).toBeGreaterThan(0);
    rebuilt += a.slice(posA, span.fromA) + b.slice(span.fromB, span.toB);
    posA = span.toA;
    posB = span.toB;
  }

  expect(a.slice(posA)).toBe(b.slice(posB));
  expect(rebuilt + a.slice(posA)).toBe(b);
}

/** The numbers (from 1) of the lines of `text` that a span marks (a span that starts at a line break marks from the next line, as the merge view reads it). */
function touched(text: string, spans: readonly Span[], side: 'A' | 'B') {
  const lines = new Set<number>();

  for (const span of spans) {
    const [start, to] = side === 'A' ? [span.fromA, span.toA] : [span.fromB, span.toB];
    const from = text[start] === '\n' && to > start ? start + 1 : start;
    const first = text.slice(0, from).split('\n').length;
    const last = text.slice(0, Math.max(from, to - 1)).split('\n').length;

    for (let line = first; line <= last; line++) {
      lines.add(line);
    }
  }

  return [...lines];
}

const numbered = (count: number, change: Record<number, string> = {}) =>
  Array.from({ length: count }, (_, index) => change[index] ?? `line ${index}`).join('\n');

describe('diffing two texts', () => {
  it('finds no change in equal texts, and a change confined to the characters that differ', () => {
    expect(diff('a\nb', 'a\nb')).toEqual([]);
    expect(diff('', '')).toEqual([]);
    expect(diff('keep\nthe old words\nend', 'keep\nthe new words\nend')).toEqual([{ fromA: 9, toA: 12, fromB: 9, toB: 12 }]);
  });

  it('finds an inserted and a deleted line at the start, the middle and the end', () => {
    const base = 'a\nb\nc';

    for (const other of ['x\na\nb\nc', 'a\nb\nx\nc', 'a\nb\nc\nx', 'a\nx\nb\nc']) {
      const spans = diff(base, other);
      const inserted = other.split('\n').indexOf('x') + 1;

      expectFaithful(base, other, spans);
      expectFaithful(other, base, diff(other, base));
      expect(touched(other, spans, 'B')).toEqual([inserted]);
      expect(diff(other, base)).toHaveLength(1);
    }
  });

  it('handles an empty side', () => {
    for (const [a, b] of [['', 'x'], ['x', ''], ['', 'x\ny'], ['x\ny', ''], ['', '\n'], ['\n', '']]) {
      expectFaithful(a, b, diff(a, b));
      expect(diff(a, b).length).toBeGreaterThan(0);
    }
  });

  it('keeps the line break at the end of one text and not the other straight', () => {
    for (const [a, b] of [['x\n', 'x'], ['x', 'x\n'], ['x\ny\n', 'x\ny'], ['x\ny', 'x\ny\nz'], ['x\ny\n', 'x\ny\nz'], ['x\ny', 'x\ny\nz\n']]) {
      expectFaithful(a, b, diff(a, b));
      expectFaithful(b, a, diff(b, a));
    }
  });

  it('shows a moved block as deleted in one place and added in the other', () => {
    const a = numbered(30);
    const lines = a.split('\n');
    const moved = [...lines.slice(0, 5), ...lines.slice(20, 25), ...lines.slice(5, 20), ...lines.slice(25)].join('\n');
    const spans = diff(a, moved);

    expectFaithful(a, moved, spans);
    expect(spans.length).toBeGreaterThanOrEqual(2);
    expect(touched(moved, spans, 'B').length).toBeLessThanOrEqual(10);
  });

  it('refines a replaced block to the characters that changed, inside the lines', () => {
    const a = 'one\nsum = alpha + beta\nthree';
    const b = 'one\nsum = alpha - beta\nthree';

    expect(diff(a, b)).toEqual([{ fromA: 16, toA: 17, fromB: 16, toB: 17 }]);
  });

  it('marks each changed line of a long run of edited lines, within a bounded work', () => {
    const a = numbered(3000);
    const b = a.replace(/line/g, 'Line');
    const started = performance.now();
    const spans = diff(a, b);

    expect(performance.now() - started).toBeLessThan(2000);
    expectFaithful(a, b, spans);
    expect(touched(a, spans, 'A')).toHaveLength(3000);
  });

  it('reports a block too big to refine as changed in its lines, not in its characters', () => {
    const a = `start\n${'a'.repeat(5000)}\nend`;
    const b = `start\n${'b'.repeat(5000)}\nend`;
    const refined: Array<[number, number]> = [];
    const spans = diffText(a, b, (x, y) => {
      refined.push([x.length, y.length]);

      return [];
    });

    expect(refined).toEqual([]);
    expectFaithful(a, b, spans);
    expect(touched(a, spans, 'A')).toEqual([2]);
  });

  it('cuts the shared start and end off a long line before looking at what differs', () => {
    const middle = 'x'.repeat(30);
    const a = `${'p'.repeat(100_000)}${middle}${'s'.repeat(100_000)}`;
    const b = `${'p'.repeat(100_000)}y${'s'.repeat(100_000)}`;
    const spans = diff(a, b);

    expect(spans).toEqual([{ fromA: 100_000, toA: 100_030, fromB: 100_000, toB: 100_001 }]);
  });

  it('does not cut a surrogate pair in two', () => {
    const a = 'say \u{1F600} now';
    const b = 'say \u{1F601} now';
    const [span] = diffText(a, b, (x, y) => [{ fromA: 0, toA: x.length, fromB: 0, toB: y.length }]);

    expect(a.slice(span.fromA, span.toA)).toBe('\u{1F600}');
    expect(b.slice(span.fromB, span.toB)).toBe('\u{1F601}');
  });

  it('is faithful on random edits of random texts', () => {
    let seed = 2024;
    const next = (limit: number) => (seed = (seed * 1103515245 + 12345) >>> 0) % limit;
    const words = ['alpha', 'beta', '}', '{', '', 'x', 'return a;'];

    for (let round = 0; round < 400; round++) {
      const lines = Array.from({ length: 1 + next(20) }, () => words[next(words.length)] + (next(3) ? '' : ' tail'));
      const other = lines.slice();

      for (let edit = next(5); edit > 0; edit--) {
        const at = next(other.length + 1);
        const kind = next(3);

        if (kind === 0) {
          other.splice(at, 0, words[next(words.length)]);
        } else if (kind === 1) {
          other.splice(at, 1);
        } else if (at < other.length) {
          other[at] += '!';
        }
      }

      const a = lines.join('\n') + (next(2) ? '\n' : '');
      const b = other.join('\n') + (next(2) ? '\n' : '');

      expectFaithful(a, b, diff(a, b));
    }
  });

  it('gives up a repetitive stretch that needs too many edits as one change (right, only coarser)', () => {
    const a = Array.from({ length: 2000 }, (_, index) => (index % 2 ? 'x' : 'y')).join('\n');
    const b = Array.from({ length: 2000 }, (_, index) => (index % 2 ? 'y' : 'x')).join('\n');
    const spans = diff(a, b);

    expectFaithful(a, b, spans);
    expect(spans.length).toBeLessThan(5);
  });

  it('still marks only the lines that changed when a repetitive stretch is too big to search for its edits', () => {
    // No line is unique and 500 replaced lines are more than the search takes on, but the lines pair up one to one.
    const a = Array.from({ length: 1500 }, () => 'same').join('\n');
    const b = Array.from({ length: 1500 }, (_, index) => (index % 3 === 1 ? 'diff' : 'same')).join('\n');
    const spans = diff(a, b);

    expectFaithful(a, b, spans);
    expect(touched(a, spans, 'A')).toHaveLength(500);
  });

  it('keeps its time on unrelated one-line texts of several MiB (the case that froze the window)', () => {
    let seed = 5;
    const random = (size: number) =>
      Array.from({ length: size / 1000 }, () => {
        let chunk = '';

        for (let i = 0; i < 1000; i++) {
          chunk += String.fromCharCode(65 + ((seed = (seed * 1103515245 + 12345) >>> 0) >>> 8) % 58);
        }

        return chunk;
      }).join('');
    const a = random(4_000_000);
    const b = random(4_000_000);
    const started = performance.now();
    const spans = diff(a, b);

    expect(performance.now() - started).toBeLessThan(1000);
    expect(spans).toHaveLength(1);
  });
});
