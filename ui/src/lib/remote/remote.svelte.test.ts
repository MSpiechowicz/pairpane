import { describe, expect, it, vi } from 'vitest';

import type { ConnectOutcome } from '$lib/ipc/bindings/ConnectOutcome';
import type { ConnectTarget } from '$lib/ipc/bindings/ConnectTarget';
import { PaneTabs } from '$lib/panes/tabs.svelte';

import { arrive, deliverRemote, entry, fakeBackend, remoteStart } from '../../../tests/panes';
import { Remote } from './remote.svelte';

const TARGET: ConnectTarget = { host: 'files', port: null, user: null, keyFile: null };
const CONNECTED: ConnectOutcome = { kind: 'connected', site: 5, label: 'ann@files', home: '/home/ann' };
const ASK: ConnectOutcome = { kind: 'secretNeeded', reason: null, secret: 'password', wrong: false, other: null };

/** Two panes (the right one active, in `/work`) and a controller whose server answers with `outcomes`, one per try. */
async function world(outcomes: (ConnectOutcome | Error)[]) {
  const answers = [...outcomes];
  const backend = fakeBackend({
    remote_hosts: () => [{ alias: 'files', hostName: 'files.example.com', user: 'ann', port: 2222 }],
    remote_connect: () => {
      const next = answers.shift();
      return next instanceof Error ? Promise.reject({ code: 'forbidden', message: next.message }) : next;
    },
  });
  const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  await panes.right.view.listing.open('/work');
  arrive(backend.last(), '/work', [entry('a.txt')]);
  const say = vi.fn();
  const remote = new Remote(backend.ipc, panes, () => 'right', say);
  const sent = () => backend.invoke.mock.calls.filter(([name]) => name === 'remote_connect').map(([, args]) => args);

  return { backend, panes, remote, say, sent };
}

/** The `remote_disconnect` calls the backend got. */
const disconnects = (backend: Awaited<ReturnType<typeof world>>['backend']) =>
  backend.invoke.mock.calls.filter(([name]) => name === 'remote_disconnect').map(([, args]) => args);

describe('Remote', () => {
  it('opens its dialog with the hosts of the SSH configuration, and none when they cannot be read', async () => {
    const { remote } = await world([]);

    await remote.show();

    expect(remote.shown).toBe(true);
    expect(remote.modal).toBe(true);
    expect(remote.hosts.map((host) => host.alias)).toEqual(['files']);
  });

  it('lists the home folder of the server in the active pane once connected, and closes the dialog', async () => {
    const { backend, panes, remote, sent } = await world([CONNECTED]);
    await remote.show();

    await remote.connect(TARGET);

    expect(sent()).toEqual([{ target: TARGET, secret: null, kind: null }]);
    expect(backend.last().args).toMatchObject({ pane: 'right', site: 5, dir: '/home/ann', child: null });
    expect(panes.left.view.listing.remote).toBeUndefined();
    expect(remote.modal).toBe(false);
    expect(remote.target).toBeUndefined();
  });

  it('asks for the password when the server wants one, and connects again with it', async () => {
    const { backend, remote, sent } = await world([ASK, CONNECTED]);
    await remote.show();

    await remote.connect(TARGET);
    expect(remote.prompt).toEqual({ secret: 'password', other: null, wrong: false, reason: null, failure: undefined, busy: false });
    expect(remote.shown).toBe(true);

    await remote.submitSecret('hunter2');

    expect(sent()).toEqual([
      { target: TARGET, secret: null, kind: null },
      { target: TARGET, secret: 'hunter2', kind: 'password' },
    ]);
    expect(backend.last().args).toMatchObject({ site: 5, dir: '/home/ann' });
    expect(remote.prompt).toBeUndefined();
    expect(JSON.stringify(remote)).not.toContain('hunter2');
  });

  it('asks again, saying the password was wrong, and for a passphrase when it is the key that wants one', async () => {
    const wrong: ConnectOutcome = { kind: 'secretNeeded', reason: null, secret: 'passphrase', wrong: true, other: null };
    const { remote, sent } = await world([{ kind: 'secretNeeded', reason: null, secret: 'passphrase', wrong: false, other: null }, wrong, CONNECTED]);
    await remote.show();
    await remote.connect(TARGET);

    await remote.submitSecret('nope');

    expect(remote.prompt).toMatchObject({ secret: 'passphrase', wrong: true, busy: false });
    await remote.submitSecret('right');
    expect(remote.prompt).toBeUndefined();
    expect(sent()).toEqual([
      { target: TARGET, secret: null, kind: null },
      { target: TARGET, secret: 'nope', kind: 'passphrase' },
      { target: TARGET, secret: 'right', kind: 'passphrase' },
    ]);
  });

  it('sends each secret with the kind the server asked for at that moment: a passphrase first, then a password', async () => {
    const passphrase: ConnectOutcome = { kind: 'secretNeeded', reason: null, secret: 'passphrase', wrong: false, other: null };
    const password: ConnectOutcome = { kind: 'secretNeeded', reason: null, secret: 'password', wrong: false, other: null };
    const { remote, sent } = await world([passphrase, password, CONNECTED]);
    await remote.show();
    await remote.connect(TARGET);
    expect(remote.prompt).toMatchObject({ secret: 'passphrase' });

    await remote.submitSecret('key-phrase');
    expect(remote.prompt).toMatchObject({ secret: 'password', wrong: false, busy: false });
    expect(JSON.stringify(remote)).not.toContain('key-phrase');

    await remote.submitSecret('account-pw');

    expect(sent()).toEqual([
      { target: TARGET, secret: null, kind: null },
      { target: TARGET, secret: 'key-phrase', kind: 'passphrase' },
      { target: TARGET, secret: 'account-pw', kind: 'password' },
    ]);
    expect(remote.prompt).toBeUndefined();
    expect(JSON.stringify(remote)).not.toContain('key-phrase');
    expect(JSON.stringify(remote)).not.toContain('account-pw');
  });

  it('never sends a secret without the question it answers', async () => {
    const { remote, sent } = await world([CONNECTED]);
    await remote.show();

    await remote.submitSecret('stray');

    expect(sent()).toEqual([]);
  });

  it('keeps the question on the screen, with the reason, when the connection fails for another one', async () => {
    const { remote } = await world([ASK, new Error('The host key was not accepted.')]);
    await remote.show();
    await remote.connect(TARGET);

    await remote.submitSecret('x');

    expect(remote.prompt).toMatchObject({ wrong: false, busy: false, failure: 'The host key was not accepted.' });
  });

  it('tells a refusal in the Connect dialog, which stays', async () => {
    const { remote, backend } = await world([new Error('Connection timed out.')]);
    await remote.show();
    const opens = backend.opens.length;

    await remote.connect(TARGET);

    expect(remote.failure).toBe('Connection timed out.');
    expect(remote.shown).toBe(true);
    expect(remote.busy).toBe(false);
    expect(backend.opens).toHaveLength(opens);
  });

  it('goes back to the Connect dialog when the question is given up', async () => {
    const { remote } = await world([ASK]);
    await remote.show();
    await remote.connect(TARGET);

    remote.cancelSecret();

    expect(remote.prompt).toBeUndefined();
    expect(remote.shown).toBe(true);
  });

  it('drops the answer of a connection that was given up while it was made', async () => {
    let answer: (outcome: ConnectOutcome) => void = () => {};
    const backend = fakeBackend({ remote_connect: () => new Promise((resolve) => (answer = resolve)) });
    const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
    const remote = new Remote(backend.ipc, panes, () => 'left', vi.fn());
    await remote.show();

    const connecting = remote.connect(TARGET);
    expect(remote.busy).toBe(true);
    remote.close();
    answer(CONNECTED);
    await connecting;

    expect(backend.opens).toHaveLength(0);
    expect(remote.modal).toBe(false);
  });

  describe('a connection that was given up while it was made, and came through', () => {
    /** The dialog is closed while `remote_connect` is in flight; the server answers `connected` only after. */
    async function abandoned() {
      let answer: (outcome: ConnectOutcome) => void = () => {};
      const backend = fakeBackend({ remote_connect: () => new Promise((resolve) => (answer = resolve)) });
      const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
      const remote = new Remote(backend.ipc, panes, () => 'left', vi.fn());
      await remote.show();

      const connecting = remote.connect(TARGET);
      remote.close();

      return {
        backend,
        panes,
        remote,
        arrives: async () => {
          answer(CONNECTED);
          await connecting;
        },
      };
    }

    it('is closed again, as nothing shows it', async () => {
      const { backend, arrives } = await abandoned();

      await arrives();

      expect(disconnects(backend)).toEqual([{ site: 5 }]);
    });

    it('is kept when a pane already shows that server', async () => {
      const { backend, panes, arrives } = await abandoned();
      await deliverRemote(backend, panes.right.view.listing, '/home/ann', [], 5, 'ann@files');

      await arrives();

      expect(disconnects(backend)).toEqual([]);
      expect(panes.right.view.listing.remote?.site).toBe(5);
    });
  });

  describe('attempts at the same server that overlap', () => {
    /** Every `remote_connect` waits for its own answer, so the tests choose the order they come in. */
    async function overlapping() {
      const answers: ((outcome: ConnectOutcome | Error) => void)[] = [];
      const backend = fakeBackend({
        remote_connect: () =>
          new Promise((resolve, reject) =>
            answers.push((outcome) => (outcome instanceof Error ? reject({ code: 'forbidden', message: outcome.message }) : resolve(outcome))),
          ),
      });
      const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
      const remote = new Remote(backend.ipc, panes, () => 'left', vi.fn());
      const attempt = async () => {
        await remote.show();
        const connecting = remote.connect(TARGET);
        return { connecting, answer: answers[answers.length - 1]! };
      };

      return { backend, panes, remote, attempt };
    }

    it('keeps a server a newer attempt answered first, when the abandoned one arrives before the pane lists', async () => {
      const { backend, panes, remote, attempt } = await overlapping();
      const first = await attempt();
      remote.close();
      const second = await attempt();

      second.answer(CONNECTED);
      await second.connecting;
      expect(panes.left.view.listing.remote).toBeUndefined();
      first.answer(CONNECTED);
      await first.connecting;

      expect(disconnects(backend)).toEqual([]);
    });

    it('keeps a server the abandoned attempt reached while the newer one, at the same server, was in flight', async () => {
      const { backend, remote, attempt } = await overlapping();
      const first = await attempt();
      remote.close();
      const second = await attempt();

      first.answer(CONNECTED);
      await first.connecting;
      expect(disconnects(backend)).toEqual([]);
      second.answer(CONNECTED);
      await second.connecting;

      expect(disconnects(backend)).toEqual([]);
    });

    it('closes the server of the abandoned attempt once the newer one is over without it', async () => {
      const { backend, remote, attempt } = await overlapping();
      const first = await attempt();
      remote.close();
      const second = await attempt();

      first.answer(CONNECTED);
      await first.connecting;
      second.answer(new Error('Connection timed out.'));
      await second.connecting;

      expect(disconnects(backend)).toEqual([{ site: 5 }]);
    });

    it('closes a different server of the abandoned attempt after the newer one is over, and keeps the one that attempt opened', async () => {
      const { backend, remote, attempt } = await overlapping();
      const first = await attempt();
      remote.close();
      const second = await attempt();

      first.answer({ ...CONNECTED, site: 8, label: 'bob@other' });
      await first.connecting;
      expect(disconnects(backend)).toEqual([]);
      second.answer(CONNECTED);
      await second.connecting;

      expect(disconnects(backend)).toEqual([{ site: 8 }]);
    });

    it('lets a later abandoned attempt close a server that was disconnected', async () => {
      const { backend, panes, remote, attempt } = await overlapping();
      await panes.left.view.listing.open('/work');
      arrive(backend.last(), '/work', [entry('a.txt')]);
      const first = await attempt();
      first.answer(CONNECTED);
      await first.connecting;
      backend.last().emit(remoteStart(5, 'ann@files', '/home/ann', 0));
      await remote.disconnect(panes.left);
      arrive(backend.last(), '/work', [entry('a.txt')]);
      expect(panes.left.view.listing.remote).toBeUndefined();
      const again = await attempt();
      remote.close();

      again.answer(CONNECTED);
      await again.connecting;

      expect(disconnects(backend)).toEqual([{ site: 5 }, { site: 5 }]);
    });
  });

  describe('switching the kind of secret', () => {
    const phrase = (other: 'password' | null = 'password', wrong = false): ConnectOutcome => ({ kind: 'secretNeeded', reason: null, secret: 'passphrase', wrong, other });
    const word = (other: 'passphrase' | null = 'passphrase', wrong = false): ConnectOutcome => ({ kind: 'secretNeeded', reason: null, secret: 'password', wrong, other });

    it('swaps the asked kind with the other one, both ways, without asking the server', async () => {
      const { remote, sent } = await world([phrase()]);
      await remote.show();
      await remote.connect(TARGET);
      expect(remote.prompt).toMatchObject({ secret: 'passphrase', other: 'password' });

      remote.switchSecret();
      expect(remote.prompt).toEqual({ secret: 'password', other: 'passphrase', wrong: false, reason: null, failure: undefined, busy: false });

      remote.switchSecret();
      expect(remote.prompt).toMatchObject({ secret: 'passphrase', other: 'password' });
      expect(sent()).toHaveLength(1);
    });

    it('sends the secret with the kind that is shown, whichever way it was switched', async () => {
      const { remote, sent } = await world([word(), CONNECTED, word(), phrase(), CONNECTED]);
      await remote.show();

      await remote.connect(TARGET);
      remote.switchSecret();
      await remote.submitSecret('key-phrase');
      expect(sent()[1]).toEqual({ target: TARGET, secret: 'key-phrase', kind: 'passphrase' });

      await remote.show();
      await remote.connect(TARGET);
      await remote.submitSecret('x');
      remote.switchSecret();
      await remote.submitSecret('account-pw');
      expect(sent().slice(3).map((call) => call?.kind)).toEqual(['password', 'password']);
      expect(JSON.stringify(remote)).not.toContain('key-phrase');
      expect(JSON.stringify(remote)).not.toContain('account-pw');
    });

    it('keeps the switch after a wrong answer while the server still reports the other kind, and drops it when it does not', async () => {
      const { remote } = await world([phrase(), phrase('password', true), phrase(null, true)]);
      await remote.show();
      await remote.connect(TARGET);

      await remote.submitSecret('nope');
      expect(remote.prompt).toMatchObject({ secret: 'passphrase', other: 'password', wrong: true });

      await remote.submitSecret('nope again');
      expect(remote.prompt).toMatchObject({ secret: 'passphrase', other: null, wrong: true });
    });

    it('keeps the reason the server gives for the kind it asks next, and clears it when the kind is switched', async () => {
      const refused: ConnectOutcome = { kind: 'secretNeeded', secret: 'password', wrong: false, other: 'passphrase', reason: 'keyNotAccepted' };
      const { remote } = await world([phrase(), refused]);
      await remote.show();
      await remote.connect(TARGET);
      expect(remote.prompt?.reason).toBeNull();

      await remote.submitSecret('key-phrase');
      expect(remote.prompt).toMatchObject({ secret: 'password', other: 'passphrase', reason: 'keyNotAccepted' });

      remote.switchSecret();
      expect(remote.prompt).toMatchObject({ secret: 'passphrase', reason: null });
    });

    it('clears the alert and the failure of the last try when it switches, and does nothing without another kind or while busy', async () => {
      const { remote, sent } = await world([word(null), word('passphrase', true), new Error('Connection lost.')]);
      await remote.show();
      await remote.connect(TARGET);

      remote.switchSecret();
      expect(remote.prompt).toMatchObject({ secret: 'password', other: null });

      await remote.submitSecret('a');
      expect(remote.prompt).toMatchObject({ wrong: true, other: 'passphrase' });
      await remote.submitSecret('b');
      expect(remote.prompt).toMatchObject({ failure: 'Connection lost.' });

      remote.switchSecret();
      expect(remote.prompt).toEqual({ secret: 'passphrase', other: 'password', wrong: false, reason: null, failure: undefined, busy: false });
      expect(sent()).toHaveLength(3);
    });
  });

  describe('disconnect', () => {
    it('closes the connection and takes every pane that was on it back to the folder it had on disk', async () => {
      const { backend, panes, remote, say } = await world([CONNECTED]);
      await remote.show();
      await remote.connect(TARGET);
      backend.last().emit(remoteStart(5, 'ann@files', '/home/ann', 0));

      await remote.disconnect(panes.right);

      expect(backend.invoke).toHaveBeenCalledWith('remote_disconnect', { site: 5 });
      expect(backend.last().args).toMatchObject({ path: '/work' });
      expect(say).not.toHaveBeenCalled();
    });

    it('says so when the pane is not on a server', async () => {
      const { backend, panes, remote, say } = await world([]);

      await remote.disconnect(panes.right);

      expect(say).toHaveBeenCalledWith('This pane is not on a server.', true);
      expect(backend.invoke).not.toHaveBeenCalledWith('remote_disconnect', expect.anything());
    });
  });
});
