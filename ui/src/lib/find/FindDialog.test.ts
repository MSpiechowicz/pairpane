import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { FindCriteria } from '$lib/ipc/bindings/FindCriteria';

import { done, findBackend, hit } from '../../../tests/find';
import FindDialog from './FindDialog.svelte';
import { Finder } from './finder.svelte';
import { blankForm, toCriteria } from './query';
import { SavedSearches } from './saved.svelte';

const WINDOWS = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)';

function open(answers: Parameters<typeof findBackend>[0] = {}) {
  const backend = findBackend(answers);
  const finder = new Finder(backend.ipc);
  const saved = new SavedSearches();
  const results = { openResults: vi.fn(async () => {}) };
  const onGoTo = vi.fn();
  finder.show('/home/ann');
  render(FindDialog, { finder, saved, results, onGoTo });
  return { ...backend, finder, saved, results, onGoTo };
}

const field = (name: string) => screen.getByRole('textbox', { name }) as HTMLInputElement;
const box = (group: string, name: string) => within(screen.getByRole('group', { name: group })).getByRole('checkbox', { name });
const start = () => fireEvent.click(screen.getByRole('button', { name: 'Start' }));
const type = (target: HTMLElement, value: string) => fireEvent.input(target, { target: { value } });
const options = () => within(screen.getByRole('listbox', { name: 'Results' })).getAllByRole('option');
const rows = () => options().map((row) => row.textContent?.replace(/\s+/g, ' ').trim());

/** A search that has found `hits`, and is done. */
async function found(s: ReturnType<typeof open>, ...hits: ReturnType<typeof hit>[]) {
  await start();
  const search = s.searches[0];
  search.emit({ kind: 'found', hits });
  search.emit(done(hits.length));
  await tick();
  return search;
}

afterEach(() => vi.restoreAllMocks());

describe('FindDialog', () => {
  it('names the folder it looks in and puts the focus on the name field', () => {
    open();

    expect(screen.getByRole('dialog', { name: 'Find files' })).toBeInTheDocument();
    expect(screen.getByText('/home/ann')).toBeInTheDocument();
    expect(field('Names')).toHaveFocus();
    expect(screen.getByRole('button', { name: 'Go to' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Feed to panel' })).toBeDisabled();
  });

  it('Start searches with exactly what the fields hold', async () => {
    const s = open();
    await type(field('Names'), '*.rs; *.toml');
    await fireEvent.click(box('Name', 'Match case'));
    await type(field('Text'), 'TODO');
    await fireEvent.click(box('Content', 'Whole word'));
    await fireEvent.change(screen.getByRole('combobox', { name: 'Encoding' }), { target: { value: 'KOI8-R' } });
    await type(field('At least'), '2');
    await fireEvent.change(screen.getByRole('combobox', { name: 'At least unit' }), { target: { value: 'MB' } });
    await type(screen.getByLabelText('To'), '2026-11-20');
    await fireEvent.click(box('Options', 'Skip ignored (.gitignore)'));

    await start();

    await vi.waitFor(() => expect(s.searches).toHaveLength(1));
    expect(s.searches[0].args.query).toEqual({
      root: '/home/ann',
      criteria: expect.objectContaining({
        names: '*.rs; *.toml',
        nameCase: true,
        text: { pattern: 'TODO', regex: false, caseSensitive: false, wholeWord: true, encoding: 'KOI8-R', anySize: false },
        minSize: 2 * 1024 ** 2,
        maxSize: null,
        modifiedTo: new Date(2026, 10, 20, 23, 59, 59, 999).getTime(),
        skipIgnored: true,
      }),
    });
  });

  it('Enter in a field starts the search', async () => {
    const s = open();

    await fireEvent.keyDown(field('Names'), { key: 'Enter' });

    await vi.waitFor(() => expect(s.searches).toHaveLength(1));
  });

  it('a form that cannot be searched says why and asks nothing', async () => {
    const s = open();
    await type(field('At least'), 'lots');

    await start();

    expect(screen.getByRole('alert')).toHaveTextContent('Type a size as a number');
    expect(s.searches).toHaveLength(0);
  });

  it('a refusal of the backend is shown, and the form can be searched again', async () => {
    const s = open({
      find_start: () => {
        throw { code: 'invalidInput', message: 'the regular expression is not valid' };
      },
    });

    await start();

    expect(await screen.findByRole('alert')).toHaveTextContent('the regular expression is not valid');
    expect(screen.getByRole('button', { name: 'Start' })).toBeEnabled();
    expect(s.finder.status).toBe('idle');
  });

  it('lists the results as they stream in, with size, date, matches and the first line, and sums up when done', async () => {
    const s = open();
    await start();
    const search = s.searches[0];

    search.emit({ kind: 'progress', folders: 4, files: 30, bytes: 2048 });
    await tick();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();
    expect(screen.getByText(/Looking… 4 folders · 30 files · 2\.0 KB read/)).toBeInTheDocument();

    search.emit({
      kind: 'found',
      hits: [hit('src/a.rs', { matches: 3, line: 'let x = TODO;', lineNumber: 12 }), hit('docs', { kind: 'dir' })],
    });
    search.emit(done(2, { skipped: 1 }));
    await tick();

    expect(rows()[0]).toContain('src/a.rs');
    expect(rows()[0]).toContain('2.0 KB');
    expect(rows()[0]).toContain('2026-');
    expect(rows()[0]).toContain('3 12: let x = TODO;');
    expect(rows()[1]).not.toContain('KB');
    expect(screen.getByRole('status')).toHaveTextContent('2 results · 1 skipped (unreadable or too big)');
    expect(screen.getByRole('button', { name: 'Start' })).toBeInTheDocument();
  });

  it('a path is text, never markup', async () => {
    const s = open();

    await found(s, hit('<img src=x onerror=alert(1)>.txt', { line: '<b>x</b>', matches: 1, lineNumber: 1 }));

    expect(document.querySelector('img, b')).toBeNull();
    expect(rows()[0]).toContain('<img src=x onerror=alert(1)>.txt');
  });

  it('a path that would pass for another is written out and so is its tooltip', async () => {
    const s = open();

    await found(s, hit('invoice\u202Efdp.exe', { inner: 'in\nner' }));

    expect(rows()[0]).toContain('invoice⟨U+202E⟩fdp.exe› in⟨U+000A⟩ner');
    expect(options()[0].textContent).not.toMatch(/[\u202E\n]/);
    expect(options()[0].querySelector('.path')).toHaveAttribute('title', 'invoice⟨U+202E⟩fdp.exe › in⟨U+000A⟩ner');
  });

  it('a line of content and the folder searched are written out too', async () => {
    const backend = findBackend();
    const finder = new Finder(backend.ipc);
    finder.show('/home/\u202Eann');
    render(FindDialog, { finder, saved: new SavedSearches(), results: { openResults: vi.fn(async () => {}) }, onGoTo: vi.fn() });
    await start();
    backend.searches[0].emit({ kind: 'found', hits: [hit('a.txt', { line: 'x\u200Bdone\u202E', matches: 1, lineNumber: 2 })] });
    await tick();

    expect(screen.getByText('/home/⟨U+202E⟩ann')).toBeInTheDocument();
    expect(options()[0].querySelector('.line')).toHaveTextContent('2: x⟨U+200B⟩done⟨U+202E⟩');
  });

  it('says it stopped at the limit, and that it was stopped', async () => {
    const s = open();
    await start();
    s.searches[0].emit(done(100000, { truncated: true }));
    await tick();
    expect(screen.getByRole('status')).toHaveTextContent('100,000 results · the search stopped at its limit: narrow it to see more');

    await start();
    s.searches[1].emit(done(0, { cancelled: true }));
    await tick();
    expect(screen.getByRole('status')).toHaveTextContent('Stopped: 0 results');
  });

  it('Stop cancels the search', async () => {
    const s = open();
    await start();
    await vi.waitFor(() => expect(s.finder.running).toBe(true));
    await tick();

    await fireEvent.click(screen.getByRole('button', { name: 'Stop' }));

    await vi.waitFor(() => expect(s.calls('find_cancel')).toEqual([{ id: 1 }]));
    expect(screen.getByRole('button', { name: 'Stop' })).toBeDisabled();
  });

  it('shows the groups of duplicates under their heads', async () => {
    const s = open();
    await fireEvent.click(box('Options', 'Duplicates'));
    await start();

    s.searches[0].emit({ kind: 'group', size: 1048576, hits: [hit('a/x.bin'), hit('b/x.bin')] });
    await tick();

    expect(screen.getByText('2 files of 1.0 MB each')).toBeInTheDocument();
    expect(options()).toHaveLength(2);
  });

  describe('going to a result', () => {
    it('the cursor starts on the first result; Go to opens it and closes the dialog', async () => {
      const s = open();
      await found(s, hit('a.txt'), hit('b/b.txt'));
      expect(options()[0]).toHaveAttribute('aria-selected', 'true');

      await fireEvent.click(screen.getByRole('button', { name: 'Go to' }));

      expect(s.onGoTo).toHaveBeenCalledWith(expect.objectContaining({ path: 'a.txt' }));
      expect(s.finder.modal).toBe(false);
    });

    it('the arrows move the cursor and Enter goes to that result', async () => {
      const s = open();
      await found(s, hit('a.txt'), hit('b/b.txt'));
      const list = screen.getByRole('listbox', { name: 'Results' });

      await fireEvent.keyDown(list, { key: 'ArrowDown' });
      await fireEvent.keyDown(list, { key: 'Enter' });

      expect(s.onGoTo).toHaveBeenCalledWith(expect.objectContaining({ path: 'b/b.txt' }));
    });

    it('a click chooses a result and a double click goes to it; the head of a group is not one', async () => {
      const s = open();
      await start();
      s.searches[0].emit({ kind: 'group', size: 5, hits: [hit('x'), hit('y')] });
      s.searches[0].emit(done(2));
      await tick();
      const [x, y] = options();
      expect(x).toHaveAttribute('aria-selected', 'true');

      await fireEvent.click(y);
      expect(y).toHaveAttribute('aria-selected', 'true');
      await fireEvent.dblClick(y);

      expect(s.onGoTo).toHaveBeenCalledWith(expect.objectContaining({ path: 'y' }));
    });
  });

  describe('feeding a panel', () => {
    it('lists the results of a finished search in the active pane and closes', async () => {
      const s = open();
      await found(s, hit('a.txt'));

      await fireEvent.click(screen.getByRole('button', { name: 'Feed to panel' }));

      await vi.waitFor(() => expect(s.results.openResults).toHaveBeenCalledWith(1));
      await vi.waitFor(() => expect(s.finder.modal).toBe(false));
    });

    it('is off while the search runs, and says that archive entries are not listed', async () => {
      const s = open();
      await start();
      s.searches[0].emit({ kind: 'found', hits: [hit('a.zip', { inner: 'x.txt' }), hit('b.txt')] });
      await tick();
      expect(screen.getByRole('button', { name: 'Feed to panel' })).toBeDisabled();

      s.searches[0].emit(done(2));
      await tick();

      expect(screen.getByRole('button', { name: 'Feed to panel' })).toBeEnabled();
      expect(screen.getByText('Entries inside archives are not listed in a panel.')).toBeInTheDocument();
    });
  });

  describe('closing', () => {
    it('Escape and Close forget the search', async () => {
      const s = open();
      await start();
      await vi.waitFor(() => expect(s.finder.running).toBe(true));

      await fireEvent.keyDown(window, { key: 'Escape' });

      expect(s.finder.modal).toBe(false);
      await vi.waitFor(() => expect(s.calls('find_forget')).toEqual([{ id: 1 }]));
    });
  });

  describe('choices that cannot go together', () => {
    it('archives switch off the text and the duplicates; text or duplicates switch off the archives', async () => {
      open();
      await fireEvent.click(box('Options', 'Inside archives'));

      expect(field('Text')).toBeDisabled();
      expect(box('Options', 'Duplicates')).toBeDisabled();

      await fireEvent.click(box('Options', 'Inside archives'));
      await type(field('Text'), 'x');
      expect(box('Options', 'Inside archives')).toBeDisabled();

      await type(field('Text'), '');
      await fireEvent.click(box('Options', 'Duplicates'));
      expect(box('Options', 'Inside archives')).toBeDisabled();
    });

    it('skipping hidden files and asking for them cannot both be set', async () => {
      open();
      await fireEvent.click(box('Options', 'Skip hidden'));

      expect(within(screen.getAllByRole('combobox', { name: 'Hidden' })[0]).getByRole('option', { name: 'Yes' })).toBeDisabled();
    });

    it('the executable test is offered where there is an execute bit', () => {
      open();

      expect(screen.getByLabelText('Executable')).toBeInTheDocument();
    });
  });

  describe('saved searches', () => {
    async function save(name: string) {
      await fireEvent.click(screen.getByRole('button', { name: 'Save as…' }));
      const input = screen.getByRole('textbox', { name: 'Name of the search' });
      await type(input, name);
      await fireEvent.keyDown(input, { key: 'Enter' });
    }

    it('Save as keeps the criteria of the form under the name, and the dialog stays open', async () => {
      const s = open();
      await type(field('Names'), '*.log');

      await save('Logs');

      expect(s.saved.list).toEqual([{ name: 'Logs', criteria: expect.objectContaining({ names: '*.log' }) }]);
      expect(screen.getByRole('combobox', { name: 'Saved search' })).toHaveValue('Logs');
      expect(s.searches).toHaveLength(0); // Enter in the name field saves, it does not search
      expect(screen.getByRole('dialog', { name: 'Find files' })).toBeInTheDocument();
    });

    it('refuses a name the backend would, and a form that cannot be searched', async () => {
      open();

      await save('   ');
      expect(screen.getByRole('alert')).toHaveTextContent('Type a name for the search.');

      await fireEvent.keyDown(screen.getByRole('textbox', { name: 'Name of the search' }), { key: 'Escape' });
      await type(field('At least'), 'x');
      await save('Bad');
      expect(screen.getByRole('alert')).toHaveTextContent('Type a size as a number');
    });

    it('Escape in the name field gives up the name, not the dialog', async () => {
      open();
      await fireEvent.click(screen.getByRole('button', { name: 'Save as…' }));

      await fireEvent.keyDown(screen.getByRole('textbox', { name: 'Name of the search' }), { key: 'Escape' });

      expect(screen.queryByRole('textbox', { name: 'Name of the search' })).not.toBeInTheDocument();
      expect(screen.getByRole('dialog', { name: 'Find files' })).toBeInTheDocument();
    });

    it('Load puts a saved search back into the fields, Delete takes it away', async () => {
      const s = open();
      const criteria = toCriteria(
        { ...blankForm(), names: '*.log', minSize: '5', minUnit: 'MB', hidden: 'no', skipIgnored: true },
        false,
      ) as FindCriteria;
      s.saved.restore([{ name: 'Big logs', criteria }]);
      await tick();
      const picker = screen.getByRole('combobox', { name: 'Saved search' });

      await fireEvent.change(picker, { target: { value: 'Big logs' } });
      await fireEvent.click(screen.getByRole('button', { name: 'Load' }));

      expect(field('Names')).toHaveValue('*.log');
      expect(field('At least')).toHaveValue('5');
      expect(screen.getByRole('combobox', { name: 'At least unit' })).toHaveValue('MB');
      expect(box('Options', 'Skip ignored (.gitignore)')).toBeChecked();

      await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));

      expect(s.saved.list).toEqual([]);
      expect(screen.getByRole('combobox', { name: 'Saved search' })).toHaveValue('');
    });
  });
});

describe('FindDialog on Windows', () => {
  it('has no executable test', () => {
    vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(WINDOWS);
    open();

    expect(screen.queryByLabelText('Executable')).not.toBeInTheDocument();
    expect(screen.getByLabelText('Hidden')).toBeInTheDocument();
  });
});
