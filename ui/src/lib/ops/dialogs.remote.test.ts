import { describe, expect, it } from 'vitest';

import { opsBackend } from '../../../tests/ops';
import { perform, specOf } from './dialogs';
import type { OpRequest } from './targets';

const request = (move: boolean, names = ['a.txt']): Extract<OpRequest, { kind: 'remote' }> => ({
  kind: 'remote',
  move,
  from: { kind: 'remote', site: 3, dir: '/srv/in' },
  to: { kind: 'folder', path: '/work' },
  names,
  shown: '/work',
});

describe('the dialog for copying or moving out of a server', () => {
  it('names the verb and the items, shows the folder here, cannot change it and offers verifying', () => {
    expect(specOf(request(false))).toMatchObject({ title: 'Copy “a.txt” to', confirm: 'Copy', initial: '/work', fixed: true, verify: true });
    expect(specOf(request(true, ['a', 'b']))).toMatchObject({ title: 'Move 2 items to', confirm: 'Move', initial: '/work', fixed: true });
  });
});

describe('performing a transfer out of a server', () => {
  it.each([
    [false, 'copy'],
    [true, 'move'],
  ] as const)('%s asks ops_remote for one %s job with the two places, the names and the verify choice', async (move, kind) => {
    const backend = opsBackend({ ops_remote: () => 7 });

    const done = await perform(backend.ipc, request(move, ['a.txt', 'sub']), '/work', true);

    expect(backend.calls('ops_remote')).toEqual([
      {
        kind,
        from: { kind: 'remote', site: 3, dir: '/srv/in' },
        to: { kind: 'folder', path: '/work' },
        names: ['a.txt', 'sub'],
        verify: true,
      },
    ]);
    expect(backend.calls('ops_start')).toEqual([]);
    expect(done).toEqual({ name: '/work' });
  });

  it('lets a refusal of the backend reject, for the dialog to show', async () => {
    const backend = opsBackend({
      ops_remote: () => {
        throw new Error('no');
      },
    });

    await expect(perform(backend.ipc, request(false), '/work', false)).rejects.toThrow('no');
  });
});
