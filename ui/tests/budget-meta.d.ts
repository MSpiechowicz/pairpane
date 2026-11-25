// What a performance budget test hands to ui/budget-reporter.ts through its `meta` (docs/performance-budgets.md).
import 'vitest';

/** What a budget test measured, in milliseconds, and its limit. */
export interface BudgetMeasure {
  name: string;
  measuredMs: number;
  budgetMs: number;
}

declare module 'vitest' {
  interface TaskMeta {
    budget?: BudgetMeasure;
  }
}
