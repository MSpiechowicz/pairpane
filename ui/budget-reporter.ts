// Reports the performance budgets the UI tests measured (ui/src/lib/palette/budget.test.ts): a line for each on the
// console and, in CI, a row in the job summary (GITHUB_STEP_SUMMARY). A test hands its measurement over in its `meta`,
// because the tests run against the browser's types and cannot write a file themselves. See docs/performance-budgets.md.
import { appendFileSync } from 'node:fs';
import { platform } from 'node:os';

import type { Reporter, TestCase } from 'vitest/node';

// The `budget` field of a test's `meta`.
import type {} from './tests/budget-meta';

export class BudgetReporter implements Reporter {
  onTestCaseResult(testCase: TestCase) {
    const budget = testCase.meta().budget;
    if (budget === undefined) {
      return;
    }

    const measured = `${budget.measuredMs.toFixed(1)} ms`;
    const limit = `≤ ${budget.budgetMs} ms`;
    const verdict = budget.measuredMs <= budget.budgetMs ? 'met' : 'MISSED';
    process.stdout.write(`budget ${budget.name}: ${measured} (budget ${limit}) ${verdict}\n`);

    const summary = process.env.GITHUB_STEP_SUMMARY;
    if (summary) {
      appendFileSync(summary, `| ${platform()} | ${budget.name} | ${measured} | ${limit} | ${verdict} |\n`);
    }
  }
}
