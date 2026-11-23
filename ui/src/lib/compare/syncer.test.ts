import { describe, expect, it, vi } from 'vitest';

import { IpcFailure } from '$lib/ipc/transport';

import { compareBackend, facts, finished, folder, MODIFIED, row, rows } from '../../../tests/compare';
import { actions, ARCHIVE, paths, planned, ROOTS, setup, started } from '../../../tests/syncer';
import { Syncer } from './syncer.svelte';

describe('Syncer scan', () => {
  it('asks for the two folders with the options, and lists the rows as they stream in', async () => {
    const s = setup();
    const scan = await started(s);

    expect(scan.args).toMatchObject({ ...ROOTS, options: { subfolders: true, content: false, hidden: false } });
    expect(s.syncer.running).toBe(true);

    scan.emit(rows(row('a.txt', 'leftOnly', facts(), null), row('b/c.txt', 'rightNewer')));
    scan.emit({ kind: 'progress', folders: 2, files: 9, bytes: 4096 });
    scan.emit(rows(row('d.txt', 'differs')));

    expect(paths(s.syncer)).toEqual(['a.txt', 'b/c.txt', 'd.txt']);
    expect(s.syncer.progress).toEqual({ folders: 2, files: 9, bytes: 4096 });
    expect(s.syncer.counts).toMatchObject({ leftOnly: 1, rightNewer: 1, differs: 1 });
  });

  it('takes the options as they are set, hidden from the app setting', async () => {
    const backend = compareBackend();
    const syncer = new Syncer(backend.ipc);
    syncer.show(ROOTS, true);
    syncer.subfolders = false;
    syncer.content = true;

    await syncer.start();

    expect(backend.scans[0].args.options).toEqual({ subfolders: false, content: true, hidden: true });
  });

  it('never asks for content with an archive on a side', async () => {
    const backend = compareBackend();
    const syncer = new Syncer(backend.ipc);
    syncer.content = true;
    syncer.show({ left: folder('/left'), right: ARCHIVE }, false);

    expect(syncer.contentAllowed).toBe(false);
    await syncer.start();

    expect(backend.scans[0].args.options.content).toBe(false);
  });

  it('events that arrive before the id does are listed all the same', async () => {
    const s = setup({ compare_start: () => new Promise(() => {}) });
    void s.syncer.start();
    await vi.waitFor(() => expect(s.scans).toHaveLength(1));

    s.scans[0].emit(rows(row('early.txt', 'leftNewer')));

    expect(paths(s.syncer)).toEqual(['early.txt']);
  });

  it('the done event gives the summary and ends the run', async () => {
    const s = setup();
    const scan = await started(s);

    scan.emit(finished(7, { skipped: 2, truncated: true }));

    expect(s.syncer.status).toBe('done');
    expect(s.syncer.summary).toEqual({ rows: 7, skipped: 2, truncated: true, cancelled: false });
  });

  it('an error event ends the run with its message', async () => {
    const s = setup();
    const scan = await started(s);

    scan.emit({ kind: 'error', error: { code: 'invalidInput', message: 'One folder is inside the other' } });

    expect(s.syncer.status).toBe('failed');
    expect(s.syncer.failure).toBe('One folder is inside the other');
  });

  it('a scan the backend refuses to start shows why and can be started again', async () => {
    const s = setup({
      compare_start: () => {
        throw new IpcFailure({ code: 'busy', message: 'two scans are running' });
      },
    });

    await s.syncer.start();

    expect(s.syncer.status).toBe('idle');
    expect(s.syncer.failure).toBe('two scans are running');
  });

  it('Stop cancels the scan by its id and keeps the rows; the done event says it was stopped', async () => {
    const s = setup();
    const scan = await started(s);
    scan.emit(rows(row('a', 'leftNewer')));

    s.syncer.stop();
    s.syncer.stop(); // asked once
    scan.emit(finished(1, { cancelled: true }));

    expect(s.calls('compare_cancel')).toEqual([{ id: 1 }]);
    expect(s.syncer.stopping).toBe(false);
    expect(s.syncer.summary?.cancelled).toBe(true);
    expect(paths(s.syncer)).toEqual(['a']);
  });

  it('Stop before the id arrives cancels the scan as soon as it has one', async () => {
    let answer: (id: number) => void = () => {};
    const s = setup({ compare_start: () => new Promise((resolve) => (answer = resolve as never)) });
    const pending = s.syncer.start();
    await vi.waitFor(() => expect(s.scans).toHaveLength(1));

    s.syncer.stop();
    answer(5);
    await pending;

    expect(s.calls('compare_cancel')).toEqual([{ id: 5 }]);
  });

  it('closing the dialog cancels a running scan and forgets it; events that still arrive are ignored', async () => {
    const s = setup();
    const scan = await started(s);

    s.syncer.close();
    scan.emit(rows(row('late', 'leftOnly')));

    expect(s.calls('compare_cancel')).toEqual([{ id: 1 }]);
    expect(s.calls('compare_forget')).toEqual([{ id: 1 }]);
    expect(s.syncer.total).toBe(0);
    expect(s.syncer.modal).toBe(false);
  });

  it('closing after the scan is done only forgets it', async () => {
    const s = setup();
    const scan = await started(s);
    scan.emit(finished(0));

    s.syncer.close();

    expect(s.calls('compare_cancel')).toEqual([]);
    expect(s.calls('compare_forget')).toEqual([{ id: 1 }]);
  });

  it('a close before the id arrives cancels and forgets the scan when it does', async () => {
    let answer: (id: number) => void = () => {};
    const s = setup({ compare_start: () => new Promise((resolve) => (answer = resolve as never)) });
    const pending = s.syncer.start();
    await vi.waitFor(() => expect(s.scans).toHaveLength(1));

    s.syncer.close();
    answer(9);
    await pending;

    expect(s.calls('compare_cancel')).toEqual([{ id: 9 }]);
    expect(s.calls('compare_forget')).toEqual([{ id: 9 }]);
  });

  it('starting again forgets the earlier scan and starts with no rows', async () => {
    const s = setup();
    const first = await started(s);
    first.emit(rows(row('old', 'leftOnly')));
    first.emit(finished(1));

    const second = await started(s);

    expect(s.calls('compare_forget')).toEqual([{ id: 1 }]);
    expect(s.syncer.total).toBe(0);
    first.emit(rows(row('stale', 'leftOnly')));
    second.emit(rows(row('new', 'leftOnly')));
    expect(paths(s.syncer)).toEqual(['new']);
  });
});

describe('Syncer actions', () => {
  it('start as the status asks: copy to the newer side, nothing for what differs or is equal', async () => {
    const s = setup();
    const scan = await started(s);
    scan.emit(
      rows(
        row('lo', 'leftOnly', facts(), null),
        row('ro', 'rightOnly', null, facts()),
        row('ln', 'leftNewer'),
        row('rn', 'rightNewer'),
        row('df', 'differs'),
        row('eq', 'equal'),
      ),
    );
    s.syncer.shown.equal = true;

    expect(actions(s.syncer)).toEqual(['toRight', 'toLeft', 'toRight', 'toLeft', 'skip', 'skip']);
  });

  it('are not offered for links, special files, a file against a folder, and equal rows', async () => {
    const s = setup();
    const scan = await started(s);
    scan.emit(
      rows(
        row('link', 'leftOnly', facts({ link: true, kind: 'other' }), null),
        row('fifo', 'leftOnly', facts({ kind: 'other' }), null),
        row('mixed', 'differs', facts({ kind: 'dir' }), facts()),
        row('eq', 'equal'),
      ),
    );
    s.syncer.shown.equal = true;

    expect(actions(s.syncer)).toEqual(['skip', 'skip', 'skip', 'skip']);
    for (let index = 0; index < 4; index += 1) {
      const entry = s.syncer.at(index)!;
      expect(s.syncer.choose(entry, 'toRight')).toBe(false);
      expect(s.syncer.choose(entry, 'toLeft')).toBe(false);
    }
  });

  it('cannot point from a side that has nothing', async () => {
    const s = setup();
    const scan = await started(s);
    scan.emit(rows(row('lo', 'leftOnly', facts(), null)));
    const entry = s.syncer.at(0)!;

    expect(s.syncer.choose(entry, 'toLeft')).toBe(false);
    expect(entry.action).toBe('toRight');
    expect(s.syncer.choose(entry, 'skip')).toBe(true);
    expect(entry.action).toBe('skip');
  });

  it('cannot point into an archive: the default is then to skip', async () => {
    const s = setup({}, { left: folder('/left'), right: ARCHIVE });
    const scan = await started(s);
    scan.emit(rows(row('lo', 'leftOnly', facts(), null), row('ro', 'rightOnly', null, facts()), row('ln', 'leftNewer')));

    expect(actions(s.syncer)).toEqual(['skip', 'toLeft', 'skip']);
    expect(s.syncer.choose(s.syncer.at(2)!, 'toRight')).toBe(false);
    expect(s.syncer.choose(s.syncer.at(2)!, 'toLeft')).toBe(true);
  });

  it('are set for the picked rows that are shown; the rows that cannot go that way keep theirs', async () => {
    const s = setup();
    const scan = await started(s);
    scan.emit(rows(row('lo', 'leftOnly', facts(), null), row('ln', 'leftNewer'), row('ro', 'rightOnly', null, facts())));
    s.syncer.pickAll(true);

    s.syncer.chooseAll('toLeft');

    expect(actions(s.syncer)).toEqual(['toRight', 'toLeft', 'toLeft']);
    expect(s.syncer.notice).toBe('1 of the rows cannot go that way.');
  });

  it('are not set for a picked row the filters hide', async () => {
    const s = setup();
    const scan = await started(s);
    scan.emit(rows(row('a.txt', 'leftNewer'), row('b.md', 'leftNewer')));
    s.syncer.pickAll(true);
    s.syncer.nameFilter = '*.txt';

    expect(s.syncer.picked).toBe(1);
    s.syncer.chooseAll('skip');
    s.syncer.nameFilter = '';

    expect(actions(s.syncer)).toEqual(['skip', 'toRight']);
  });
});

describe('Syncer filters', () => {
  it('show what passes the status filter, equal rows only on request', async () => {
    const s = setup();
    const scan = await started(s);
    scan.emit(rows(row('a', 'leftOnly', facts(), null), row('b', 'equal'), row('c', 'differs')));

    expect(paths(s.syncer)).toEqual(['a', 'c']);

    s.syncer.shown.equal = true;
    s.syncer.shown.leftOnly = false;

    expect(paths(s.syncer)).toEqual(['b', 'c']);
    expect(s.syncer.total).toBe(3);
  });

  it('match the name filter against the name, not the folders above it; a blank one matches all', async () => {
    const s = setup();
    const scan = await started(s);
    scan.emit(rows(row('docs.txt/a.md', 'leftNewer'), row('docs/b.TXT', 'leftNewer'), row('c.rs', 'leftNewer')));

    s.syncer.nameFilter = '*.txt; *.rs';
    expect(paths(s.syncer)).toEqual(['docs/b.TXT', 'c.rs']);

    s.syncer.nameFilter = '  ';
    expect(paths(s.syncer)).toHaveLength(3);
  });
});

describe('Syncer apply', () => {
  it('adds up the copies of each direction, a one-sided folder with its subtree, and the files it replaces', async () => {
    const { syncer } = await planned();

    expect(syncer.totals).toEqual({ toRight: { items: 2, bytes: 105 }, toLeft: { items: 1, bytes: 3000 }, overwrites: 1, removeLeft: { items: 0, bytes: 0 }, removeRight: { items: 0, bytes: 0 } });
  });

  it('asks for a confirmation only when something is to be copied', async () => {
    const { syncer } = await planned();

    expect(syncer.ask()).toBe(true);
    expect(syncer.confirming).toBe(true);

    for (let index = 0; index < syncer.count; index += 1) {
      syncer.choose(syncer.at(index)!, 'skip');
    }

    expect(syncer.ask()).toBe(false);
    expect(syncer.confirming).toBe(false);
  });

  it('queues one job with the copies as the scan saw them, and none for the rows left alone', async () => {
    const s = await planned();

    await s.syncer.apply();

    expect(s.calls('ops_sync')).toEqual([
      {
        ...ROOTS,
        items: [
          { path: 'new.txt', direction: 'toRight', source: { kind: 'file', size: 100, modified: MODIFIED }, target: null },
          { path: 'dir', direction: 'toLeft', source: { kind: 'dir', size: 0, modified: MODIFIED }, target: null },
          {
            path: 'over.txt',
            direction: 'toRight',
            source: { kind: 'file', size: 5, modified: 2000 },
            target: { kind: 'file', size: 7, modified: 1000 },
          },
        ],
        removals: [],
        permanent: false,
      },
    ]);
  });

  it('synchronizes only the rows shown: what the filters hide is neither counted nor sent', async () => {
    const { syncer, calls } = await planned();

    syncer.nameFilter = 'new.*; skip.txt';

    expect(syncer.totals).toEqual({ toRight: { items: 1, bytes: 100 }, toLeft: { items: 0, bytes: 0 }, overwrites: 0, removeLeft: { items: 0, bytes: 0 }, removeRight: { items: 0, bytes: 0 } });
    expect(syncer.withheld).toBe(2);

    await syncer.apply();

    expect(calls('ops_sync')).toHaveLength(1);
    expect(calls('ops_sync')[0]).toMatchObject({ items: [{ path: 'new.txt', direction: 'toRight' }] });
  });

  it('counts every hidden row with an action, asks for nothing when all of those are hidden, and none when nothing is', async () => {
    const { syncer } = await planned();

    syncer.nameFilter = '*.txt';
    syncer.shown.leftOnly = false;
    syncer.shown.rightOnly = false;
    syncer.shown.leftNewer = false;

    expect(syncer.withheld).toBe(3);
    expect(syncer.ask()).toBe(false);

    syncer.shown.equal = true;
    syncer.nameFilter = '';
    syncer.shown.leftOnly = true;
    syncer.shown.rightOnly = true;
    syncer.shown.leftNewer = true;

    expect(syncer.withheld).toBe(0);
  });

  it('rejects with the refusal of the backend, so that the dialog can show it', async () => {
    const s = setup({
      ops_sync: () => {
        throw new IpcFailure({ code: 'invalidInput', message: 'too many items' });
      },
    });
    const scan = await started(s);
    scan.emit(rows(row('a', 'leftNewer')));

    await expect(s.syncer.apply()).rejects.toThrow('too many items');
  });
});

describe('Syncer stale rows', () => {
  async function scanned() {
    const s = setup();
    const scan = await started(s);
    scan.emit(rows(row('a.txt', 'leftNewer')));
    scan.emit(finished(1));
    return s;
  }

  it.each([
    ['subfolders', (syncer: Syncer) => (syncer.subfolders = false)],
    ['content', (syncer: Syncer) => (syncer.content = true)],
    ['hidden', (syncer: Syncer) => (syncer.hidden = true)],
  ])('are marked when %s changes after the scan: the rows stay, nothing is asked for', async (_name, change) => {
    const { syncer } = await scanned();
    expect(syncer.stale).toBe(false);

    change(syncer);

    expect(syncer.stale).toBe(true);
    expect(paths(syncer)).toEqual(['a.txt']);
    expect(syncer.ask()).toBe(false);
    expect(syncer.confirming).toBe(false);
  });

  it('are not marked when the option is set back, or before there is a scan', async () => {
    const fresh = setup();
    fresh.syncer.hidden = true;
    expect(fresh.syncer.stale).toBe(false);

    const { syncer } = await scanned();
    syncer.hidden = true;
    syncer.hidden = false;

    expect(syncer.stale).toBe(false);
    expect(syncer.ask()).toBe(true);
  });

  it('are marked when an option changes while the scan runs', async () => {
    const s = setup();
    await started(s);

    s.syncer.subfolders = false;

    expect(s.syncer.stale).toBe(true);
  });

  it('are made fresh by the next Start, with the new options', async () => {
    const s = await scanned();
    s.syncer.content = true;
    expect(s.syncer.stale).toBe(true);

    await started(s);

    expect(s.syncer.stale).toBe(false);
    expect(s.scans[1].args.options).toEqual({ subfolders: true, content: true, hidden: false });
  });
});
