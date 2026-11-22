import { describe, expect, it } from 'vitest';

import { blankRule, copyRule, insertToken, wholeNumber } from './rule';

describe('blankRule', () => {
  it('keeps every name and extension, counting from 1', () => {
    expect(blankRule()).toMatchObject({ name: '[N]', ext: '[E]', search: '', case: 'keep', counter: { start: 1, step: 1, digits: 1 } });
  });

  it('a copy shares nothing with the rule it was made from', () => {
    const rule = blankRule();
    const copy = copyRule(rule);

    copy.counter.start = 9;

    expect(rule.counter.start).toBe(1);
  });
});

describe('wholeNumber', () => {
  it('reads digits only, up to the largest the backend can read', () => {
    expect(wholeNumber('0', 255)).toBe(0);
    expect(wholeNumber(' 12 ', 255)).toBe(12);
    expect(wholeNumber('255', 255)).toBe(255);
    expect(wholeNumber('256', 255)).toBeUndefined();
    expect(wholeNumber('', 255)).toBeUndefined();
    expect(wholeNumber('-1', 255)).toBeUndefined();
    expect(wholeNumber('1.5', 255)).toBeUndefined();
    expect(wholeNumber('1e3', 255)).toBeUndefined();
  });
});

describe('insertToken', () => {
  it('puts the token at the caret and leaves the caret after it', () => {
    expect(insertToken('ab', 1, 1, '[C]')).toEqual({ value: 'a[C]b', caret: 4 });
    expect(insertToken('', 0, 0, '[N]')).toEqual({ value: '[N]', caret: 3 });
  });

  it('replaces what is selected', () => {
    expect(insertToken('a-b-c', 1, 4, '[N]')).toEqual({ value: 'a[N]c', caret: 4 });
  });
});
