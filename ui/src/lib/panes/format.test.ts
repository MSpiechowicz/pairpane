import { describe, expect, it } from 'vitest';

import { displayName } from './format';

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

  it('writes out C0 and C1 controls, newlines and DEL included, each as its own code', () => {
    expect(displayName('a\nb\u0000c\u007Fd\u0085e')).toBe('a⟨U+000A⟩b⟨U+0000⟩c⟨U+007F⟩d⟨U+0085⟩e');
  });
});
