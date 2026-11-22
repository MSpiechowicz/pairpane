import { describe, expect, it, vi } from 'vitest';

import type { CommandName } from '$lib/ipc/commands';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { Passwords } from './passwords.svelte';

const required = (message = 'The archive needs a password.') => new IpcFailure({ code: 'passwordRequired', message });

/**
 * A backend whose `archive_password` takes `good` only and whose guarded commands refuse with a password failure
 * until a password was taken (a later one asks again as `wrong` says).
 */
function world(good = 'pw') {
  let taken = false;
  const calls: [string, unknown][] = [];
  const invoke = vi.fn(async (name: CommandName, args: unknown) => {
    calls.push([name, args]);
    if (name === 'archive_password') {
      taken = (args as { password: string }).password === good;
      return taken;
    }

    if (name === 'ops_extract' || name === 'archive_open' || name === 'archive_view_open') {
      if (!taken) {
        throw required();
      }
    }

    return name === 'view_close' ? null : 1;
  });
  const backend = { invoke } as unknown as Transport;
  const passwords = new Passwords(backend);

  return { passwords, ipc: passwords.wrap(backend), calls: (name: string) => calls.filter(([called]) => called === name).map(([, args]) => args) };
}

const extract = { archive: '/a/x.zip', dir: '', names: [], to: '/out' };

describe('Passwords', () => {
  it('passes the commands that need no password straight through', async () => {
    const { ipc, passwords } = world();

    await ipc.invoke('view_close', { session: 1 });

    expect(passwords.prompt).toBeUndefined();
  });

  it('asks when a command needs one, names the archive, and runs the command again once it was taken', async () => {
    const { ipc, passwords, calls } = world();

    const started = ipc.invoke('ops_extract', extract);
    await vi.waitFor(() => expect(passwords.prompt).toMatchObject({ archive: '/a/x.zip', name: 'x.zip', reason: 'needed' }));
    expect(passwords.modal).toBe(true);

    await passwords.submit('pw');

    await expect(started).resolves.toBe(1);
    expect(passwords.prompt).toBeUndefined();
    expect(calls('ops_extract')).toHaveLength(2);
    expect(calls('archive_password')).toEqual([{ archive: '/a/x.zip', password: 'pw' }]);
  });

  it('asks again as wrong when the backend refuses the password, and keeps the command waiting', async () => {
    const { ipc, passwords, calls } = world();
    const started = ipc.invoke('archive_open', { pane: 'left', archive: '/a/x.zip', dir: '', showHidden: false, onEvent: () => {} });
    await vi.waitFor(() => expect(passwords.prompt).toBeDefined());

    await passwords.submit('nope');

    expect(passwords.prompt).toMatchObject({ reason: 'wrong', busy: false });
    expect(calls('archive_open')).toHaveLength(1);

    await passwords.submit('pw');
    await expect(started).resolves.toBe(1);
  });

  it('reads the reason from the failure: a password that was given and did not open the archive is wrong', async () => {
    const ipc = {
      invoke: vi.fn(async () => {
        throw required('The password is wrong.');
      }),
    } as unknown as Transport;
    const passwords = new Passwords(ipc);

    void passwords.wrap(ipc).invoke('archive_view_open', { archive: '/a/x.zip', dir: '', name: 'f' }).catch(() => {});

    await vi.waitFor(() => expect(passwords.prompt?.reason).toBe('wrong'));
  });

  it('giving up fails the command with its own failure and asks nothing more', async () => {
    const { ipc, passwords, calls } = world();
    const started = ipc.invoke('ops_extract', extract);
    await vi.waitFor(() => expect(passwords.prompt).toBeDefined());

    passwords.cancel();

    await expect(started).rejects.toMatchObject({ code: 'passwordRequired' });
    expect(passwords.prompt).toBeUndefined();
    expect(calls('ops_extract')).toHaveLength(1);
    expect(calls('archive_password')).toEqual([]);
  });

  it('asks once for the commands that wait on the same archive, and each of them runs again', async () => {
    const { ipc, passwords, calls } = world();
    const first = ipc.invoke('ops_extract', extract);
    const second = ipc.invoke('archive_view_open', { archive: '/a/x.zip', dir: '', name: 'f' });
    await vi.waitFor(() => expect(calls('ops_extract')).toHaveLength(1));
    await vi.waitFor(() => expect(calls('archive_view_open')).toHaveLength(1));

    await passwords.submit('pw');
    await Promise.all([first, second]);

    expect(calls('archive_password')).toHaveLength(1);
    expect(calls('ops_extract')).toHaveLength(2);
    expect(calls('archive_view_open')).toHaveLength(2);
  });

  it('asks for one archive at a time', async () => {
    const { ipc, passwords } = world();
    const first = ipc.invoke('ops_extract', extract);
    const second = ipc.invoke('ops_extract', { ...extract, archive: '/a/y.7z' });
    await vi.waitFor(() => expect(passwords.prompt?.archive).toBe('/a/x.zip'));

    await passwords.submit('pw');
    await first;
    await vi.waitFor(() => expect(passwords.prompt?.archive).toBe('/a/y.7z'));

    passwords.cancel();
    await expect(second).rejects.toMatchObject({ code: 'passwordRequired' });
  });

  it('does not keep the password: the question holds none, and a failure of the check is shown for another try', async () => {
    const ipc = {
      invoke: vi.fn(async (name: string) => {
        if (name === 'archive_password') {
          throw new IpcFailure({ code: 'io', message: 'Could not read the archive.' });
        }

        throw required();
      }),
    } as unknown as Transport;
    const passwords = new Passwords(ipc);
    void passwords.wrap(ipc).invoke('ops_extract', extract).catch(() => {});
    await vi.waitFor(() => expect(passwords.prompt).toBeDefined());

    await passwords.submit('secret');

    expect(passwords.prompt).toMatchObject({ busy: false, failure: 'Could not read the archive.' });
    expect(JSON.stringify(passwords.prompt)).not.toContain('secret');
  });
});
