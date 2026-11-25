// The palette's performance budget of the quality gate (#24): it must feel instant with the most it ever holds, every
// command and MOST_NAMES names of the active folder. See docs/performance-budgets.md. Each test leaves what it measured in
// its `meta`; ui/budget-reporter.ts prints it and, in CI, adds it to the job summary.
import { render } from '@testing-library/svelte';
import { describe, expect, it, type TaskMeta } from 'vitest';

import { Palette as PaletteModel } from './model.svelte';
import Palette from './Palette.svelte';
import { MOST_NAMES } from './sources';

/** The longest the matches of one query may take: what a keystroke waits for. */
const MATCH_BUDGET_MS = 50;
/** The longest opening the palette may take, from nothing to its drawn list. */
const OPEN_BUDGET_MS = 100;
/** Runs of each measurement; the median counts, so one slow run on a shared runner does not fail it. */
const RUNS = 7;

const names = Array.from({ length: MOST_NAMES }, (_, index) => `photo-${String(index).padStart(5, '0')}-holiday.jpeg`);
const folders = Array.from({ length: 50 }, (_, index) => `/home/me/projects/folder-${index}`);
const sources = { folders: () => folders, names: () => names };

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
}

function record(meta: TaskMeta, name: string, measuredMs: number, budgetMs: number) {
  meta.budget = { name, measuredMs, budgetMs };
}

describe('palette budget', () => {
  it(`matches a query over every command and ${MOST_NAMES} names within budget`, ({ task }) => {
    const palette = new PaletteModel(sources);
    palette.show();
    // A first query warms the JIT, as the palette is warm after its first use.
    palette.type('hol');
    expect(palette.items.length).toBeGreaterThan(0);

    // Each query differs from the one before, so no run reads an answer worked out already.
    const queries = ['phot', 'copy', '12345', 'holjp', 'zzzz', 'fold', 'eday'];
    const times = queries.slice(0, RUNS).map((query) => {
      const start = performance.now();
      palette.type(query);
      void palette.items;
      return performance.now() - start;
    });

    const measured = median(times);
    record(task.meta, `palette: match over ${MOST_NAMES} names`, measured, MATCH_BUDGET_MS);
    expect(measured).toBeLessThanOrEqual(MATCH_BUDGET_MS);
  });

  it('opens, drawing its list, within budget', ({ task }) => {
    const times = Array.from({ length: RUNS }, () => {
      const palette = new PaletteModel(sources);
      const start = performance.now();
      palette.show();
      const { getAllByRole, unmount } = render(Palette, { palette, onTake: () => {} });
      expect(getAllByRole('option').length).toBeGreaterThan(0);
      const took = performance.now() - start;
      unmount();
      return took;
    });

    const measured = median(times);
    record(task.meta, 'palette: open', measured, OPEN_BUDGET_MS);
    expect(measured).toBeLessThanOrEqual(OPEN_BUDGET_MS);
  });
});
