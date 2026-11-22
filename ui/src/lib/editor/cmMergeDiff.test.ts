import { Chunk } from '@codemirror/merge';
import { ChangeSet, Text } from '@codemirror/state';
import { describe, expect, it } from 'vitest';

import { MERGE_DIFF } from './cmMerge';

const texts = (a: string, b: string) => [Text.of(a.split('\n')), Text.of(b.split('\n'))] as const;

/** The chunks the merge view makes for two texts with our diff, and with its own (as it was set before). */
const chunksOf = (a: string, b: string, config: object = MERGE_DIFF) => {
  const [ta, tb] = texts(a, b);

  return Chunk.build(ta, tb, config);
};

const lineOf = (text: string, at: number) => text.slice(0, at).split('\n').length;

/** How many lines of A the chunks cover. */
const linesMarked = (a: string, chunks: readonly Chunk[]) =>
  chunks.reduce((sum, chunk) => sum + lineOf(a, chunk.endA) - lineOf(a, chunk.fromA) + 1, 0);

/** A seeded generator, so a failure is the same every time. */
function random(seed: number) {
  let state = seed;

  return (limit: number) => (state = (state * 1103515245 + 12345) >>> 0) % limit;
}

const word = (next: (limit: number) => number, length: number) => Array.from({ length }, () => String.fromCharCode(97 + next(26))).join('');

/** Source-like text: unique lines of about 40 characters. */
function sourceLines(next: (limit: number) => number, bytes: number) {
  const lines: string[] = [];

  for (let size = 0; size < bytes; ) {
    const line = `  const ${word(next, 6)} = ${word(next, 8)}(${word(next, 5)}, ${word(next, 4)});`;
    lines.push(line);
    size += line.length + 1;
  }

  return lines;
}

describe('the merge view with our diff', () => {
  it('chunks small texts as its own diff does (within a few alternative alignments of repeated lines)', () => {
    const next = random(7);
    const words = ['alpha', 'beta', 'gamma', 'delta', 'x', '}', '{', 'return', 'const a = 1;', '', ' ', 'foo(bar)', 'if (x) {'];
    const line = () => Array.from({ length: 1 + next(4) }, () => words[next(words.length)]).join(' ');
    let different = 0;
    let oursMarked = 0;
    let theirsMarked = 0;

    for (let round = 0; round < 400; round++) {
      const lines = Array.from({ length: 1 + next(25) }, line);
      const other = lines.slice();

      for (let edit = next(5); edit > 0; edit--) {
        const at = next(other.length + 1);
        const kind = next(3);

        if (kind === 0) {
          other.splice(at, 0, line());
        } else if (kind === 1) {
          other.splice(at, 1);
        } else if (at < other.length) {
          other[at] = `${other[at].replace(/\w/, 'Q')}${next(2) ? ' z' : ''}`;
        }
      }

      const a = lines.join('\n') + (next(4) ? '' : '\n');
      const b = other.join('\n') + (next(4) ? '' : '\n');
      const ours = chunksOf(a, b);
      const theirs = chunksOf(a, b, { scanLimit: 1000 });
      const key = (chunks: readonly Chunk[]) => chunks.map((chunk) => [chunk.fromA, chunk.toA, chunk.fromB, chunk.toB].join()).join(' ');

      oursMarked += linesMarked(a, ours);
      theirsMarked += linesMarked(a, theirs);

      if (key(ours) !== key(theirs)) {
        different++;
      }
    }

    expect(different).toBeLessThanOrEqual(25);
    expect(oursMarked).toBeLessThanOrEqual(theirsMarked * 1.05);
  });

  it('keeps a dozen spread edits in a 40 KiB source file as a dozen chunks, not one (the stretch it used to give up on)', () => {
    const next = random(4242);
    const lines = sourceLines(next, 40 * 1024);
    const step = Math.floor(lines.length / 12);
    const edited = lines.map((line, index) => (index % step === step >> 1 ? line.replace('const', 'let') : line));
    const [a, b] = [lines.join('\n'), edited.join('\n')];
    const chunks = chunksOf(a, b);
    const edits = edited.filter((line, index) => line !== lines[index]).length;

    expect(chunks).toHaveLength(edits);
    expect(linesMarked(a, chunks)).toBe(edits);
  });

  it('keeps an edit every 100 lines of a 2 MiB file as one chunk each, quickly', () => {
    const next = random(11);
    const lines = sourceLines(next, 2 * 1024 * 1024);
    const edited = lines.map((line, index) => (index % 100 === 50 ? `${line} // changed` : line));
    const [a, b] = [lines.join('\n'), edited.join('\n')];
    const started = performance.now();
    const chunks = chunksOf(a, b);
    const took = performance.now() - started;
    const edits = Math.floor((lines.length + 49) / 100);

    expect(took).toBeLessThan(2000);
    expect(chunks).toHaveLength(edits);
    expect(linesMarked(a, chunks)).toBe(edits);
  }, 20_000);

  it('keeps a data file with every 50th row changed as a chunk per changed row', () => {
    const rows = Array.from({ length: 2500 }, (_, index) => `${index},0,0,0,1,0.0,0.0,OK`);
    const changed = rows.map((row, index) => (index % 50 ? row : row.replace('OK', 'FAIL')));

    expect(chunksOf(rows.join('\n'), changed.join('\n'))).toHaveLength(50);
  });

  it('builds two unrelated one-line files of 4 MiB at once, and again after a keystroke', () => {
    const next = random(5);
    const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    const base64 = (size: number) => Array.from({ length: size / 1000 }, () => Array.from({ length: 1000 }, () => alphabet[next(64)]).join('')).join('');
    const [a, b] = [base64(4_000_000), base64(4_000_000)];
    const [ta, tb] = texts(a, b);
    let started = performance.now();
    const chunks = Chunk.build(ta, tb, MERGE_DIFF);
    const build = performance.now() - started;

    expect(chunks).toHaveLength(1);
    expect(build).toBeLessThan(1000);

    const change = ChangeSet.of([{ from: ta.length >> 1, insert: 'q' }], ta.length);
    started = performance.now();
    Chunk.updateA(chunks, change.apply(ta), tb, change, MERGE_DIFF);

    expect(performance.now() - started).toBeLessThan(500);
  }, 20_000);

  it('recomputes the chunks near an edit, with the same result as building them again', () => {
    const next = random(3);
    const lines = sourceLines(next, 64 * 1024);
    const edited = lines.map((line, index) => (index % 120 === 60 ? `${line} // changed` : line));
    const [ta, tb] = texts(lines.join('\n'), edited.join('\n'));
    const before = Chunk.build(ta, tb, MERGE_DIFF);
    const change = ChangeSet.of([{ from: ta.line(300).from, insert: 'x' }], ta.length);
    const after = change.apply(ta);
    const updated = Chunk.updateA(before, after, tb, change, MERGE_DIFF);
    const rebuilt = Chunk.build(after, tb, MERGE_DIFF);

    expect(updated.map((chunk) => [chunk.fromA, chunk.toA, chunk.fromB, chunk.toB])).toEqual(
      rebuilt.map((chunk) => [chunk.fromA, chunk.toA, chunk.fromB, chunk.toB]),
    );
  });
});
