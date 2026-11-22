import type { CaseChange } from '$lib/ipc/bindings/CaseChange';
import type { CasePart } from '$lib/ipc/bindings/CasePart';
import type { RenameCounter } from '$lib/ipc/bindings/RenameCounter';
import type { RenameRule } from '$lib/ipc/bindings/RenameRule';

/** The rule a dialog starts with: every name and extension as it is (`[N].[E]`), counting from 1. */
export function blankRule(): RenameRule {
  return {
    name: '[N]',
    ext: '[E]',
    counter: { start: 1, step: 1, digits: 1 },
    search: '',
    replace: '',
    regex: false,
    matchCase: false,
    firstOnly: false,
    case: 'keep',
    casePart: 'both',
  };
}

/** A copy of `rule` that shares nothing with it, for a form to change. */
export function copyRule(rule: RenameRule): RenameRule {
  return { ...rule, counter: { ...rule.counter } };
}

/** The placeholders the buttons put into a template; the label says what each stands for. */
export const PLACEHOLDERS: readonly { token: string; label: string }[] = [
  { token: '[N]', label: 'Name' },
  { token: '[E]', label: 'Extension' },
  { token: '[C]', label: 'Counter' },
  { token: '[Y]', label: 'Year' },
  { token: '[M]', label: 'Month' },
  { token: '[D]', label: 'Day' },
  { token: '[h]', label: 'Hour' },
  { token: '[m]', label: 'Minute' },
  { token: '[s]', label: 'Second' },
  { token: '[YMD]', label: 'Date (year-month-day)' },
  { token: '[hms]', label: 'Time (hour-minute-second)' },
];

export const CASES: readonly { value: CaseChange; label: string }[] = [
  { value: 'keep', label: 'Keep' },
  { value: 'lower', label: 'lower case' },
  { value: 'upper', label: 'UPPER CASE' },
  { value: 'title', label: 'Title Case' },
];

export const CASE_PARTS: readonly { value: CasePart; label: string }[] = [
  { value: 'both', label: 'Name and extension' },
  { value: 'name', label: 'Name' },
  { value: 'ext', label: 'Extension' },
];

/**
 * The counter's fields and the largest value each can hold for the backend to read it (the backend's own limits are
 * narrower and say so in a message; these only keep a number from being refused before it is looked at).
 */
export const COUNTER_FIELDS: readonly { key: keyof RenameCounter; label: string; max: number }[] = [
  { key: 'start', label: 'Start', max: 4_294_967_295 },
  { key: 'step', label: 'Step', max: 4_294_967_295 },
  { key: 'digits', label: 'Digits', max: 255 },
];

/** The whole number a counter field holds, or `undefined` while it holds none (empty, a fraction, negative, too large). */
export function wholeNumber(text: string, max: number): number | undefined {
  if (!/^\d+$/.test(text.trim())) {
    return undefined;
  }

  const value = Number(text);
  return value <= max ? value : undefined;
}

/** `value` with `token` in place of the text from `start` to `end`, and where the caret goes after it. */
export function insertToken(value: string, start: number, end: number, token: string): { value: string; caret: number } {
  return { value: value.slice(0, start) + token + value.slice(end), caret: start + token.length };
}
