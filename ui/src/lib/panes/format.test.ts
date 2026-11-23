import { describe, expect, it } from 'vitest';

import { displayName, displayText } from './format';

describe('displayName', () => {
  it('leaves an ordinary name, with spaces, accents and right-to-left letters, as it is', () => {
    expect(displayName('docs/Zażółć gęślą.txt')).toBe('docs/Zażółć gęślą.txt');
    expect(displayName('שלום.txt')).toBe('שלום.txt');
  });

  it('writes out an override that makes a name read as another', () => {
    expect(displayName('invoice\u202Efdp.exe')).toBe('invoice⟨U+202E⟩fdp.exe');
  });

  it.each(['\u202A', '\u202B', '\u202C', '\u202D', '\u202E', '\u2066', '\u2067', '\u2068', '\u2069', '\u200E', '\u200F', '\u061C'])(
    'writes out a bidirectional character (%j)',
    (char) => {
      const code = char.codePointAt(0)!.toString(16).toUpperCase().padStart(4, '0');

      expect(displayName(`a${char}b`)).toBe(`a⟨U+${code}⟩b`);
    },
  );

  it.each(['\u200B', '\u200C', '\u200D', '\u2060', '\uFEFF', '\u2028', '\u2029', '\u00AD', '\u2062', '\u{E0041}'])(
    'writes out an invisible character (%j), so that it cannot hide in a name',
    (char) => {
      const code = char.codePointAt(0)!.toString(16).toUpperCase().padStart(4, '0');

      expect(displayName(`a${char}b`)).toBe(`a⟨U+${code}⟩b`);
    },
  );

  it('writes out the joiner inside an emoji sequence too: the price of no invisible character at all', () => {
    expect(displayName('👩\u200D💻')).toBe('👩⟨U+200D⟩💻');
  });

  it('writes out the characters of a name that is shown a second time without changing it', () => {
    expect(displayName(displayName('a\u200Eb'))).toBe('a⟨U+200E⟩b');
  });

  it('writes out C0 and C1 controls, newlines and DEL included, each as its own code', () => {
    expect(displayName('a\nb\u0000c\u007Fd\u0085e')).toBe('a⟨U+000A⟩b⟨U+0000⟩c⟨U+007F⟩d⟨U+0085⟩e');
  });
});

describe('displayText', () => {
  it('keeps line breaks and tabs, and writes out the other controls and the reordering characters', () => {
    expect(displayText('a\tb\nc\u202Ed\u0007')).toBe('a\tb\nc⟨U+202E⟩d⟨U+0007⟩');
  });
});
