// Guard: an untrusted name, path or backend message is never put on the screen as it is. A file name can carry a
// right-to-left override (`txt.exe` behind it reads as another name), so each template that shows one writes it
// through `displayName`. This reads every component's template and lists the interpolations that show such a value
// without it, so that a new component cannot bring the gap back.
import { describe, expect, it } from 'vitest';

const sources = import.meta.glob('../src/**/*.svelte', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

/** What is untrusted: names and paths of the user's files, what the backend says about them, what a file says of itself. */
const UNTRUSTED =
  /\.(name|path|inner|linkTarget|suggested|root|message|failure|error|notice|note|label)\b|(^|[^.\w$])(message|failure|error|notice)\b/;

/** The attributes a screen reader or a tooltip says aloud: they show what is in them as the text does. */
const SPOKEN = new Set(['title', 'aria-label', 'alt', 'placeholder', 'aria-description']);

/** `file: expression` that shows something that is not untrusted although it looks like it, and why. */
const ALLOWED: Record<string, string> = {
  'ops/AttributesDialog.svelte: row.label': 'the fixed label of a permission row',
  'ops/OpDialog.svelte: spec.label': 'the fixed name of the field a dialog asks for (`Target folder`)',
  'ops/PasswordDialog.svelte: message': 'one of two fixed sentences',
  'ops/PasswordDialog.svelte: notice': 'a fixed sentence, or nothing',
  'viewer/HexView.svelte: row.label': 'the offset of a row in hex digits',
  'diff/HexDiffView.svelte: row.label': 'the offset of a row in hex digits',
};

interface Shown {
  expression: string;
  line: number;
}

/** The expression inside the braces that open at `from`, and where they close. */
function balanced(source: string, from: number): { expression: string; end: number } {
  let depth = 0;
  for (let at = from; at < source.length; at += 1) {
    if (source[at] === '{') {
      depth += 1;
    } else if (source[at] === '}') {
      depth -= 1;
      if (depth === 0) {
        return { expression: source.slice(from + 1, at), end: at };
      }
    }
  }

  return { expression: source.slice(from + 1), end: source.length };
}

/** The template of a component: no script, no style, no comments (their lengths are kept, so that lines stay). */
function templateOf(source: string): string {
  const blank = (match: string) => match.replace(/[^\n]/g, ' ');
  return source
    .replace(/<script[\s\S]*?<\/script>/g, blank)
    .replace(/<style[\s\S]*?<\/style>/g, blank)
    .replace(/<!--[\s\S]*?-->/g, blank);
}

/**
 * What the template of a component shows from an expression: the text it writes (`{shown.name}`) and the spoken
 * attributes of its elements (`aria-label="Close {label}"`). Values of inputs, handlers and the props of
 * components are not shown here, and the blocks (`{#if}`, `{@const}`) are not text.
 */
export function shownIn(source: string): Shown[] {
  const template = templateOf(source);
  const found: Shown[] = [];
  const lineAt = (at: number) => template.slice(0, at).split('\n').length;

  let at = 0;
  let tag: { shows: boolean; attribute: string | undefined; quote: string | undefined } | undefined;
  while (at < template.length) {
    const char = template[at];

    if (char === '{') {
      const { expression, end } = balanced(template, at);
      const plain = !/^\s*[#:/@]/.test(expression);
      const spoken = tag !== undefined && tag.shows && tag.attribute !== undefined && SPOKEN.has(tag.attribute);
      if (plain && (tag === undefined || spoken)) {
        found.push({ expression: expression.trim(), line: lineAt(at) });
      }
      if (tag !== undefined && tag.quote === undefined) {
        tag.attribute = undefined;
      }
      at = end + 1;
      continue;
    }

    if (tag === undefined) {
      const name = /^<([A-Za-z][\w:.-]*)/.exec(template.slice(at, at + 64));
      if (char === '<' && name) {
        tag = { shows: /^[a-z]/.test(name[1]) && !name[1].includes(':'), attribute: undefined, quote: undefined };
        at += name[0].length;
        continue;
      }
    } else if (tag.quote !== undefined) {
      if (char === tag.quote) {
        tag.quote = undefined;
        tag.attribute = undefined;
      }
    } else if (char === '"' || char === "'") {
      tag.quote = char;
    } else if (char === '>') {
      tag = undefined;
    } else {
      const attribute = /^([^\s=<>/"'{}]+)=/.exec(template.slice(at, at + 64));
      if (attribute) {
        tag.attribute = attribute[1];
        at += attribute[0].length;
        continue;
      }
    }

    at += 1;
  }

  return found;
}

/** The expressions that show an untrusted value without `displayName`. */
export function unwrapped(source: string): Shown[] {
  return shownIn(source).filter(({ expression }) => UNTRUSTED.test(expression) && !expression.includes('displayName('));
}

describe('names on the screen', () => {
  it('every template shows names, paths and messages through displayName', () => {
    const missed = Object.entries(sources).flatMap(([path, source]) => {
      const file = path.replace('../src/lib/', '');

      return unwrapped(source)
        .filter(({ expression }) => !(`${file}: ${expression}` in ALLOWED))
        .map(({ expression, line }) => `${file}:${line} {${expression}}`);
    });

    expect(missed).toEqual([]);
  });

  it('the multi-rename dialog and the saved bar are among the components it reads', () => {
    const files = Object.keys(sources).map((path) => path.replace('../src/lib/', ''));

    expect(files).toEqual(
      expect.arrayContaining([
        'rename/RenameDialog.svelte',
        'rename/RuleFields.svelte',
        'rename/PreviewTable.svelte',
        'saved/SavedBar.svelte',
      ]),
    );
    // The names of the items and of the saved presets are shown, so those templates have to be written through displayName.
    expect(shownIn(sources['../src/lib/rename/PreviewTable.svelte']).map(({ expression }) => expression)).toEqual(
      expect.arrayContaining(['displayName(item.from)', 'displayName(item.to)']),
    );
    expect(shownIn(sources['../src/lib/saved/SavedBar.svelte']).map(({ expression }) => expression)).toContain('displayName(kept)');
  });

  it('the synchronize dialog, its table and its confirmation are among the components it reads', () => {
    const files = Object.keys(sources).map((path) => path.replace('../src/lib/', ''));
    const shown = (file: string) => shownIn(sources[`../src/lib/${file}`]).map(({ expression }) => expression);

    expect(files).toEqual(
      expect.arrayContaining(['compare/SyncDialog.svelte', 'compare/SyncTable.svelte', 'compare/SyncConfirm.svelte', 'compare/CompareTools.svelte']),
    );
    // The paths of the rows and of the two folders, what the scan and the job say, and the row marks are shown, so each is written through displayName.
    expect(shown('compare/SyncTable.svelte')).toEqual(expect.arrayContaining(['displayName(item.row.path)']));
    expect(shown('compare/SyncDialog.svelte')).toEqual(
      expect.arrayContaining(['displayName(place(roots.left))', 'displayName(place(roots.right))', 'displayName(syncer.failure)', 'displayName(syncer.notice)']),
    );
    expect(shownIn(sources['../src/lib/panes/PaneRow.svelte']).map(({ expression }) => expression)).toContain('MARK_TEXT[mark]');
  });

  it('the file diff, its headers, its toolbar and its two views are among the components it reads', () => {
    const files = Object.keys(sources).map((path) => path.replace('../src/lib/', ''));
    const shown = (file: string) => shownIn(sources[`../src/lib/${file}`]).map(({ expression }) => expression);

    expect(files).toEqual(
      expect.arrayContaining([
        'diff/DiffOverlay.svelte',
        'diff/DiffSideHeader.svelte',
        'diff/DiffToolbar.svelte',
        'diff/DiffNote.svelte',
        'diff/DiffFrame.svelte',
        'diff/TextDiffView.svelte',
        'diff/HexDiffView.svelte',
        'editor/EditBanners.svelte',
      ]),
    );
    // The names of the two files, the notes (also what the hex comparison says and what it failed with) and what a side failed with are shown, so each is written through displayName.
    expect(shown('diff/DiffSideHeader.svelte')).toEqual(expect.arrayContaining(['displayName(session.name)', 'displayName(session.encoding)']));
        expect(shown('diff/DiffToolbar.svelte')).toContain('displayName(summary)');
    expect(shown('diff/HexDiffView.svelte')).toEqual(expect.arrayContaining(['displayName(hex.left.name)', 'displayName(hex.right.name)']));
    expect(shown('diff/DiffNote.svelte')).toContain('displayName(text)');
    expect(shown('editor/EditBanners.svelte')).toEqual(expect.arrayContaining(['displayName(session.note)', 'displayName(session.failure)']));
  });

  it('looks at the components that are there, and every exception is still needed', () => {
    const files = Object.keys(sources).map((path) => path.replace('../src/lib/', ''));
    expect(files.length).toBeGreaterThan(40);

    const used = Object.entries(sources).flatMap(([path, source]) =>
      unwrapped(source).map(({ expression }) => `${path.replace('../src/lib/', '')}: ${expression}`),
    );
    expect(Object.keys(ALLOWED).sort()).toEqual([...new Set(used)].filter((key) => key in ALLOWED).sort());
  });

  describe('the scan', () => {
    const find = (template: string) => unwrapped(`<script lang="ts">let a = 1;</script>\n${template}`).map(({ expression }) => expression);

    it('finds a name in text, in a title and in an aria-label that is written as it is', () => {
      expect(find('<span>{entry.name}</span>')).toEqual(['entry.name']);
      expect(find('<span title={hit.path}>x</span>')).toEqual(['hit.path']);
      expect(find('<button aria-label="Close {tab.name}">x</button>')).toEqual(['tab.name']);
      expect(find('<p role="alert">{failure}</p>')).toEqual(['failure']);
      expect(find('<p\n  class="a"\n>\n  {session.error}\n</p>')).toEqual(['session.error']);
    });

    it('accepts what goes through displayName', () => {
      expect(find('<span>{displayName(entry.name)}</span>')).toEqual([]);
      expect(find('<button aria-label="Close {displayName(tab.name)}">x</button>')).toEqual([]);
    });

    it('leaves the values that go to the backend and what only a component shows', () => {
      expect(find('<input bind:value={rename.name} value={entry.name} />')).toEqual([]);
      expect(find('<button onclick={() => open(entry.path)}>Open</button>')).toEqual([]);
      expect(find('<Dialog path={dialog.path} title={session.name} />')).toEqual([]);
      expect(find('{#each rows as row (row.path)}{/each}{#if x.error}{/if}{@const a = b.name}')).toEqual([]);
    });

    it('is not fooled by arrows and comparisons in the braces of a tag', () => {
      expect(find('<button disabled={a > b} onclick={() => go(x.name)}>{entry.name}</button>')).toEqual(['entry.name']);
    });
  });
});
