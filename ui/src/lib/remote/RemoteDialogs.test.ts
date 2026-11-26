import { fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import type { ConnectOutcome } from '$lib/ipc/bindings/ConnectOutcome';
import type { SecretReason } from '$lib/ipc/bindings/SecretReason';
import { PaneTabs } from '$lib/panes/tabs.svelte';

import { fakeBackend } from '../../../tests/panes';
import RemoteDialogs from './RemoteDialogs.svelte';
import { Remote } from './remote.svelte';

/** The dialogs on a controller whose server answers with `outcomes`, one per try. */
async function open(outcomes: ConnectOutcome[] = []) {
  const answers = [...outcomes];
  const backend = fakeBackend({
    remote_hosts: () => [{ alias: 'work', hostName: 'work.example.com', user: 'ann', port: 2222 }],
    remote_connect: () => answers.shift(),
  });
  const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
  const remote = new Remote(backend.ipc, panes, () => 'left', vi.fn());
  render(RemoteDialogs, { remote });
  await remote.show();
  await tick();

  const sent = () => backend.invoke.mock.calls.filter(([name]) => name === 'remote_connect').map(([, args]) => args);
  const type = (label: string, value: string) => fireEvent.input(screen.getByLabelText(label), { target: { value } });
  return { remote, sent, type };
}

describe('the Connect dialog', () => {
  it('has the host, user, port and key file fields, and suggests the hosts of the configuration', async () => {
    await open();

    expect(screen.getByRole('dialog', { name: 'Connect to SFTP server' })).toBeInTheDocument();
    for (const label of ['Host', 'User', 'Port', 'Key file']) {
      expect(screen.getByLabelText(label)).toBeInTheDocument();
    }

    expect(document.querySelector('datalist option')).toHaveAttribute('value', 'work');
  });

  it('shows the user and port a named host has as the empty fields’ placeholders', async () => {
    const { type } = await open();

    await type('Host', 'work');

    expect(screen.getByLabelText('User')).toHaveAttribute('placeholder', 'ann');
    expect(screen.getByLabelText('Port')).toHaveAttribute('placeholder', '2222');
  });

  it('sends what was typed, nothing for the fields left empty', async () => {
    const { type, sent } = await open();
    await type('Host', 'work');
    await type('Port', '2200');

    await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));

    expect(sent()).toEqual([{ target: { host: 'work', port: 2200, user: null, keyFile: null }, secret: null, kind: null }]);
  });

  it('refuses a host left empty and a port that is none, without asking the server', async () => {
    const { type, sent } = await open();

    await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    expect(screen.getByRole('alert')).toHaveTextContent('Enter the host to connect to.');

    await type('Host', 'work');
    await type('Port', 'ssh');
    await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    expect(screen.getByRole('alert')).toHaveTextContent('The port is a number from 1 to 65535.');
    expect(sent()).toEqual([]);
  });

  it('closes with Cancel', async () => {
    const { remote } = await open();

    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(remote.shown).toBe(false);
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});

describe('the password question of a server', () => {
  it('asks for the password, sends it with the same target, and says when it was wrong', async () => {
    const { type, sent } = await open([
      { kind: 'secretNeeded', reason: null, secret: 'password', wrong: false, other: null },
      { kind: 'secretNeeded', reason: null, secret: 'password', wrong: true, other: null },
    ]);
    await type('Host', 'work');
    await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    await tick();

    expect(await screen.findByRole('dialog', { name: 'Password for “work”' })).toBeInTheDocument();
    expect(screen.getByText('The server asks for a password.')).toBeInTheDocument();

    const field = screen.getByLabelText('Password');
    await fireEvent.input(field, { target: { value: 'hunter2' } });
    await fireEvent.keyDown(field, { key: 'Enter' });
    await tick();

    expect(sent()[1]).toEqual({ target: expect.objectContaining({ host: 'work' }), secret: 'hunter2', kind: 'password' });
    expect(await screen.findByText('The password is wrong. Try again.')).toHaveAttribute('role', 'alert');
    expect((screen.getByLabelText('Password') as HTMLInputElement).value).toBe('');
  });

  it('asks for a passphrase when it is the key that wants one', async () => {
    const { type } = await open([{ kind: 'secretNeeded', reason: null, secret: 'passphrase', wrong: false, other: null }]);
    await type('Host', 'work');
    await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));

    expect(await screen.findByRole('dialog', { name: 'Passphrase for “work”' })).toBeInTheDocument();
    expect(screen.getByLabelText('Passphrase')).toBeInTheDocument();
  });

  it('sends the passphrase as a passphrase, and the password the server asks for next as a password', async () => {
    const { type, sent } = await open([
      { kind: 'secretNeeded', reason: null, secret: 'passphrase', wrong: false, other: null },
      { kind: 'secretNeeded', reason: null, secret: 'password', wrong: false, other: null },
      { kind: 'connected', site: 1, label: 'ann@work', home: '/home/ann' },
    ]);
    await type('Host', 'work');
    await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));

    const phrase = await screen.findByLabelText('Passphrase');
    await fireEvent.input(phrase, { target: { value: 'key-phrase' } });
    await fireEvent.keyDown(phrase, { key: 'Enter' });
    const word = await screen.findByLabelText('Password');
    await fireEvent.input(word, { target: { value: 'account-pw' } });
    await fireEvent.keyDown(word, { key: 'Enter' });
    await tick();

    expect(sent().map(({ secret, kind }) => [secret, kind])).toEqual([
      [null, null],
      ['key-phrase', 'passphrase'],
      ['account-pw', 'password'],
    ]);
  });

  it('goes back to the Connect dialog, with what was typed, when it is cancelled', async () => {
    const { type } = await open([{ kind: 'secretNeeded', reason: null, secret: 'password', wrong: false, other: null }]);
    await type('Host', 'work');
    await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    await screen.findByRole('dialog', { name: 'Password for “work”' });

    const asking = screen.getByRole('dialog', { name: 'Password for “work”' });
    await fireEvent.click(asking.querySelector('button') as HTMLButtonElement); // Cancel is the first button

    expect(screen.queryByRole('dialog', { name: 'Password for “work”' })).toBeNull();
    expect((screen.getByLabelText('Host') as HTMLInputElement).value).toBe('work');
  });
});

describe('switching the kind of secret in the dialog', () => {
  const ask = (secret: 'password' | 'passphrase', other: 'password' | 'passphrase' | null, wrong = false, reason: SecretReason | null = null): ConnectOutcome => ({ kind: 'secretNeeded', secret, other, wrong, reason });
  const connect = async (type: (label: string, value: string) => Promise<unknown>) => {
    await type('Host', 'work');
    await fireEvent.click(screen.getByRole('button', { name: 'Connect' }));
    await tick();
  };

  it('offers the password instead of the passphrase, and switches title, label and message with the field emptied', async () => {
    const { type, sent } = await open([ask('passphrase', 'password', true)]);
    await connect(type);
    const field = await screen.findByLabelText('Passphrase');
    await fireEvent.input(field, { target: { value: 'half' } });

    expect(screen.getByText('The passphrase is wrong. Try again.')).toHaveAttribute('role', 'alert');
    await fireEvent.click(screen.getByRole('button', { name: 'Use password instead' }));

    expect(screen.getByRole('dialog', { name: 'Password for “work”' })).toBeInTheDocument();
    expect(screen.getByText('The server asks for a password.')).not.toHaveAttribute('role');
    expect((screen.getByLabelText('Password') as HTMLInputElement).value).toBe('');
    expect(screen.queryByRole('alert')).toBeNull();
    expect(sent()).toHaveLength(1);
  });

  it('offers the key’s passphrase instead of the password, and the way back', async () => {
    const { type } = await open([ask('password', 'passphrase')]);
    await connect(type);
    await screen.findByLabelText('Password');

    await fireEvent.click(screen.getByRole('button', { name: "Use the key's passphrase instead" }));
    expect(screen.getByRole('dialog', { name: 'Passphrase for “work”' })).toBeInTheDocument();
    expect(screen.getByText('The private key is protected by a passphrase.')).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Use password instead' }));
    expect(screen.getByRole('dialog', { name: 'Password for “work”' })).toBeInTheDocument();
  });

  it('sends what is typed with the kind that is shown, and keeps the switch after a wrong answer', async () => {
    const { type, sent } = await open([ask('password', 'passphrase'), ask('passphrase', 'password', true), { kind: 'connected', site: 1, label: 'ann@work', home: '/home/ann' }]);
    await connect(type);
    await screen.findByLabelText('Password');
    await fireEvent.click(screen.getByRole('button', { name: "Use the key's passphrase instead" }));

    const phrase = screen.getByLabelText('Passphrase');
    await fireEvent.input(phrase, { target: { value: 'key-phrase' } });
    await fireEvent.keyDown(phrase, { key: 'Enter' });
    expect(await screen.findByText('The passphrase is wrong. Try again.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Use password instead' })).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: 'Use password instead' }));
    const word = screen.getByLabelText('Password');
    await fireEvent.input(word, { target: { value: 'account-pw' } });
    await fireEvent.keyDown(word, { key: 'Enter' });
    await tick();

    expect(sent().map((call) => call?.kind)).toEqual([null, 'passphrase', 'password']);
  });

  it('says the key was not accepted next to the password question, and drops it when switched to the passphrase', async () => {
    const { type } = await open([ask('password', 'passphrase', false, 'keyNotAccepted')]);
    await connect(type);
    await screen.findByLabelText('Password');

    expect(screen.getByRole('alert')).toHaveTextContent('The key opened, but the server did not accept it.');
    expect(screen.getByText('The server asks for a password.')).toBeInTheDocument();

    await fireEvent.click(screen.getByRole('button', { name: "Use the key's passphrase instead" }));
    expect(screen.queryByText('The key opened, but the server did not accept it.')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows no such line when the server gives no reason, and a wrong answer still shows its own alert', async () => {
    const { type } = await open([ask('password', null), ask('password', null, true, 'keyNotAccepted')]);
    await connect(type);
    await screen.findByLabelText('Password');
    expect(screen.queryByText('The key opened, but the server did not accept it.')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();

    const field = screen.getByLabelText('Password');
    await fireEvent.input(field, { target: { value: 'nope' } });
    await fireEvent.keyDown(field, { key: 'Enter' });

    expect(await screen.findByText('The password is wrong. Try again.')).toHaveAttribute('role', 'alert');
  });

  it('turns the input off while a try is checked, and empties what got in when the server turns the question over', async () => {
    let answer: (outcome: ConnectOutcome) => void = () => undefined;
    const pending = new Promise<ConnectOutcome>((resolve) => {
      answer = resolve;
    });
    const answers: (ConnectOutcome | Promise<ConnectOutcome>)[] = [ask('passphrase', 'password'), pending];
    const backend = fakeBackend({
      remote_hosts: () => [],
      remote_connect: () => answers.shift(),
    });
    const panes = { left: new PaneTabs(backend.ipc, 'left'), right: new PaneTabs(backend.ipc, 'right') };
    const remote = new Remote(backend.ipc, panes, () => 'left', vi.fn());
    render(RemoteDialogs, { remote });
    await remote.show();
    await tick();
    await connect((label, value) => fireEvent.input(screen.getByLabelText(label), { target: { value } }));

    const phrase = (await screen.findByLabelText('Passphrase')) as HTMLInputElement;
    expect(phrase).not.toBeDisabled();
    await fireEvent.input(phrase, { target: { value: 'key-phrase' } });
    await fireEvent.keyDown(phrase, { key: 'Enter' });
    await tick();

    expect(screen.getByLabelText('Passphrase')).toBeDisabled();
    await fireEvent.input(screen.getByLabelText('Passphrase'), { target: { value: 'typed-while-busy' } });

    answer(ask('password', 'passphrase'));
    const word = (await screen.findByLabelText('Password')) as HTMLInputElement;
    await tick();

    expect(word).not.toBeDisabled();
    expect(word.value).toBe('');
  });

  it('has no switch button when the server names no other kind', async () => {
    const { type } = await open([ask('password', null)]);
    await connect(type);
    await screen.findByLabelText('Password');

    expect(screen.queryByRole('button', { name: /instead/ })).toBeNull();
  });
});
