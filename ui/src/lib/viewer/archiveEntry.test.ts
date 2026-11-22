import { describe, expect, it, vi } from 'vitest';

import type { ViewOpened } from '$lib/ipc/bindings/ViewOpened';
import type { Transport } from '$lib/ipc/transport';

import { ViewSession } from './session.svelte';

const opened: ViewOpened = { session: 4, name: 'readme', size: 3, modified: null, kind: { kind: 'text' }, encoding: 'UTF-8' };

describe('ViewSession on an entry of an archive', () => {
  it('opens it through the archive and closes the session it got, as for a file', async () => {
    const invoke = vi.fn(async (name: string) => (name === 'archive_view_open' ? opened : null));
    const session = new ViewSession({ invoke } as unknown as Transport, '/work/a.zip', 'readme', undefined, 'docs');
    await vi.waitFor(() => expect(session.status).toBe('ready'));

    session.close();

    expect(invoke).toHaveBeenCalledWith('archive_view_open', { archive: '/work/a.zip', dir: 'docs', name: 'readme' });
    expect(invoke).not.toHaveBeenCalledWith('view_open', expect.anything());
    expect(invoke).toHaveBeenCalledWith('view_close', { session: 4 });
  });

  it('is closed on the backend when it is closed before the answer', async () => {
    let answer: (value: ViewOpened) => void = () => {};
    const invoke = vi.fn((name: string) => (name === 'archive_view_open' ? new Promise((resolve) => (answer = resolve)) : Promise.resolve(null)));
    const session = new ViewSession({ invoke } as unknown as Transport, '/work/a.zip', 'readme', undefined, '');

    session.close();
    answer(opened);

    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('view_close', { session: 4 }));
  });
});
