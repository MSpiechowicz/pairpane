/** The encodings the viewer offers (the names `encoding_rs` knows them by, which is what the backend takes back). */
export const ENCODINGS: readonly string[] = [
  'UTF-8',
  'UTF-16LE',
  'UTF-16BE',
  'windows-1250',
  'windows-1251',
  'windows-1252',
  'windows-1253',
  'windows-1254',
  'windows-1255',
  'windows-1256',
  'windows-1257',
  'windows-1258',
  'ISO-8859-2',
  'ISO-8859-3',
  'ISO-8859-4',
  'ISO-8859-5',
  'ISO-8859-6',
  'ISO-8859-7',
  'ISO-8859-8',
  'ISO-8859-10',
  'ISO-8859-13',
  'ISO-8859-14',
  'ISO-8859-15',
  'ISO-8859-16',
  'IBM866',
  'KOI8-R',
  'KOI8-U',
  'macintosh',
  'x-mac-cyrillic',
  'windows-874',
  'GBK',
  'gb18030',
  'Big5',
  'EUC-JP',
  'ISO-2022-JP',
  'Shift_JIS',
  'EUC-KR',
];

/** What a file that is not text (or says nothing) is read as until the user picks another. */
export const DEFAULT_ENCODING = 'UTF-8';

/** The list for the switcher: the offered encodings, and the detected one in front when it is not among them. */
export function encodingChoices(current: string): readonly string[] {
  return current === '' || ENCODINGS.includes(current) ? ENCODINGS : [current, ...ENCODINGS];
}
