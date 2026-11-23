import { describe, expect, it } from 'vitest';

import { facts, folder, MODIFIED, row, rows } from '../../../tests/compare';
import { actions, ARCHIVE, planned, setup, started } from '../../../tests/syncer';

describe('Syncer mirror', () => {
  it('a mirror to the right copies what the left has and removes what only the right has; the job carries the removals', async () => {
    const s = await planned();

    s.syncer.mirror('toRight');

    expect(actions(s.syncer)).toEqual(['toRight', 'remove', 'toRight', 'toRight']);
    expect(s.syncer.totals.removeRight).toEqual({ items: 1, bytes: 3000 });
    expect(s.syncer.removed).toEqual([{ side: 'right', path: 'dir' }]);

    s.syncer.permanent = true;
    const before = Date.now();
    await s.syncer.apply();

    // When the scan began: a folder with anything newer in it is kept by the job.
    const [removal] = (s.calls('ops_sync')[0] as { removals: { scanned: number }[] }).removals;
    expect(removal.scanned).toBeGreaterThan(0);
    expect(removal.scanned).toBeLessThanOrEqual(before);

    expect(s.calls('ops_sync')[0]).toMatchObject({
      removals: [{ path: 'dir', side: 'right', seen: { kind: 'dir', size: 0, modified: MODIFIED }, files: 3, bytes: 3000 }],
      permanent: true,
    });
    expect(s.calls('ops_sync')[0]).toMatchObject({ items: [{ path: 'new.txt' }, { path: 'over.txt' }, { path: 'skip.txt' }] });
  });

  it('a mirror to the left removes what only the left has, and asking again starts with the Trash', async () => {
    const s = await planned();
    s.syncer.permanent = true;

    s.syncer.mirror('toLeft');

    expect(actions(s.syncer)).toEqual(['remove', 'toLeft', 'toLeft', 'toLeft']);
    expect(s.syncer.totals.removeLeft).toEqual({ items: 1, bytes: 100 });
    expect(s.syncer.ask()).toBe(true);
    expect(s.syncer.permanent).toBe(false);
  });

  it('removes only a file or folder that one side alone has, never from an archive, and says how many rows refused', async () => {
    const s = setup({}, { left: folder('/left'), right: ARCHIVE });
    const scan = await started(s);
    scan.emit(
      rows(
        row('mine.txt', 'leftOnly', facts(), null),
        row('theirs.txt', 'rightOnly', null, facts()),
        row('link', 'leftOnly', facts({ link: true, kind: 'other' }), null),
        row('both.txt', 'leftNewer'),
      ),
    );
    for (let index = 0; index < s.syncer.count; index += 1) {
      s.syncer.at(index)!.picked = true;
    }

    s.syncer.chooseAll('remove');

    // From the archive, from a link and from both sides nothing is removed: those rows keep their defaults.
    expect(actions(s.syncer)).toEqual(['remove', 'toLeft', 'skip', 'skip']);
    expect(s.syncer.notice).toBe('3 of the rows cannot go that way.');

    s.syncer.mirror('toLeft');

    expect(actions(s.syncer)).toEqual(['remove', 'toLeft', 'skip', 'toLeft']);
    expect(s.syncer.notice).toBe('1 of the rows cannot go that way.');
  });

});
