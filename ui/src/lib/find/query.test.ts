import { describe, expect, it } from 'vitest';

import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';

import { blankForm, blocked, type FindForm, toCriteria, toForm } from './query';

const form = (changes: Partial<FindForm> = {}): FindForm => ({ ...blankForm(), ...changes });

describe('toCriteria', () => {
  it('a blank form asks for everything: every name, nothing else', () => {
    expect(toCriteria(blankForm(), false)).toEqual({
      names: '',
      nameRegex: false,
      namePath: false,
      nameCase: false,
      text: null,
      minSize: null,
      maxSize: null,
      modifiedFrom: null,
      modifiedTo: null,
      hidden: 'any',
      readonly: 'any',
      executable: 'any',
      skipHidden: false,
      skipIgnored: false,
      oneFilesystem: false,
      archives: false,
      duplicates: false,
    });
  });

  it('takes the names trimmed and the content with its switches and encoding', () => {
    const criteria = toCriteria(
      form({ names: ' *.rs; *.toml ', nameRegex: true, namePath: true, nameCase: true, text: 'TODO ', textCase: true, wholeWord: true, encoding: 'windows-1250', anySize: true }),
      false,
    );

    expect(criteria).toMatchObject({
      names: '*.rs; *.toml',
      nameRegex: true,
      namePath: true,
      nameCase: true,
      text: { pattern: 'TODO ', regex: false, caseSensitive: true, wholeWord: true, encoding: 'windows-1250', anySize: true },
    });
  });

  it('turns sizes with their units into bytes', () => {
    const criteria = toCriteria(form({ minSize: '1.5', minUnit: 'KB', maxSize: '2', maxUnit: 'GB' }), false);

    expect(criteria).toMatchObject({ minSize: 1536, maxSize: 2 * 1024 ** 3 });
  });

  it('refuses a size that is no number, a negative one, and a smallest above the largest', () => {
    expect(toCriteria(form({ minSize: 'big' }), false)).toBe('Type a size as a number, like 10 or 1.5.');
    expect(toCriteria(form({ maxSize: '-1' }), false)).toBe('Type a size as a number, like 10 or 1.5.');
    expect(toCriteria(form({ minSize: '2', minUnit: 'MB', maxSize: '1', maxUnit: 'MB' }), false)).toBe(
      'The smallest size is above the largest.',
    );
  });

  it('reads the dates in the local calendar: from the first moment of its day, to the last of its', () => {
    const criteria = toCriteria(form({ from: '2026-11-20', to: '2026-11-20' }), false);

    expect(criteria).toMatchObject({
      modifiedFrom: new Date(2026, 10, 20, 0, 0, 0, 0).getTime(),
      modifiedTo: new Date(2026, 10, 21, 0, 0, 0, 0).getTime() - 1,
    });
    expect((criteria as { modifiedTo: number }).modifiedTo).toBe(new Date(2026, 10, 20, 23, 59, 59, 999).getTime());
  });

  it('either date alone is a bound on one side', () => {
    expect(toCriteria(form({ from: '2026-01-01' }), false)).toMatchObject({ modifiedTo: null });
    expect(toCriteria(form({ to: '2026-01-01' }), false)).toMatchObject({ modifiedFrom: null });
  });

  it('refuses a date that is not a day and a first date after the last', () => {
    const message = 'Type a date as year-month-day, like 2026-11-20.';

    expect(toCriteria(form({ from: 'yesterday' }), false)).toBe(message);
    expect(toCriteria(form({ to: '2026-02-31' }), false)).toBe(message);
    expect(toCriteria(form({ from: '2026-11-21', to: '2026-11-20' }), false)).toBe('The first date is after the last.');
  });

  it('the executable test is left out on Windows, where there is no such thing', () => {
    expect(toCriteria(form({ executable: 'yes' }), true)).toMatchObject({ executable: 'any' });
    expect(toCriteria(form({ executable: 'yes' }), false)).toMatchObject({ executable: 'yes' });
  });

  it('refuses what the backend would: archives with duplicates or text, skipping hidden files and asking for them', () => {
    expect(toCriteria(form({ archives: true, duplicates: true }), false)).toMatch(/Archives are searched by name only/);
    expect(toCriteria(form({ archives: true, text: 'x' }), false)).toMatch(/Archives are searched by name only/);
    expect(toCriteria(form({ skipHidden: true, hidden: 'yes' }), false)).toBe('Hidden files are skipped and asked for at once.');
  });
});

describe('blocked', () => {
  it('archives switch off the text and the duplicates, and are off themselves when either is set', () => {
    expect(blocked(form({ archives: true }))).toMatchObject({ text: true, duplicates: true, archives: false });
    expect(blocked(form({ duplicates: true }))).toMatchObject({ archives: true, text: false });
    expect(blocked(form({ text: 'x' }))).toMatchObject({ archives: true, duplicates: false });
  });

  it('skipping hidden files and asking for them are one or the other', () => {
    expect(blocked(form({ skipHidden: true }))).toMatchObject({ hiddenYes: true });
    expect(blocked(form({ hidden: 'yes' }))).toMatchObject({ skipHidden: true });
    expect(blocked(blankForm())).toEqual({ text: false, archives: false, duplicates: false, skipHidden: false, hiddenYes: false });
  });
});

describe('toForm', () => {
  it('gives back the form that made the criteria, in the biggest unit that holds a size', () => {
    const typed = form({
      names: '*.log',
      text: 'error',
      textRegex: true,
      encoding: 'KOI8-R',
      minSize: '512',
      minUnit: 'B',
      maxSize: '3',
      maxUnit: 'MB',
      from: '2026-03-04',
      to: '2026-03-09',
      hidden: 'no',
      oneFilesystem: true,
    });

    const loaded = toForm(toCriteria(typed, false) as FindCriteria);

    expect(loaded).toEqual(typed);
  });
});
