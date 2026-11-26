import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import type { Transport } from '$lib/ipc/transport';

import ArchivePasswordDialog from './ArchivePasswordDialog.svelte';
import { Passwords, type Prompt } from './passwords.svelte';

const prompt = (extra: Partial<Prompt> = {}): Prompt => ({
  archive: '/a/x.zip',
  name: 'x.zip',
  reason: 'needed',
  failure: undefined,
  busy: false,
  ...extra,
});

function open(shown: Prompt) {
  const passwords = new Passwords({ invoke: vi.fn() } as unknown as Transport);
  render(ArchivePasswordDialog, { passwords, prompt: shown });
}

describe('ArchivePasswordDialog', () => {
  it('names the archive and says why it asks', () => {
    open(prompt());

    expect(screen.getByRole('dialog', { name: 'Password for “x.zip”' })).toBeInTheDocument();
    expect(screen.getByText('This archive needs a password.')).toBeInTheDocument();
  });

  it('names an archive that reorders text with its characters written out, and the failure too', () => {
    open(prompt({ name: 'invoice‮fdp.zip', failure: 'Cannot read a‮txt.exe' }));

    expect(screen.getByRole('dialog', { name: 'Password for “invoice⟨U+202E⟩fdp.zip”' })).toBeInTheDocument();
    expect(screen.getByRole('alert')).toHaveTextContent('Cannot read a⟨U+202E⟩txt.exe');
  });

  it('is an alert that says the password is wrong when the one given was', () => {
    open(prompt({ reason: 'wrong' }));

    expect(screen.getByText('The password is wrong. Try again.')).toHaveAttribute('role', 'alert');
  });

  it('hands what was typed to the question, and a cancel too', async () => {
    const passwords = { submit: vi.fn(), cancel: vi.fn() };
    render(ArchivePasswordDialog, { passwords: passwords as unknown as Passwords, prompt: prompt() });

    const field = screen.getByLabelText('Password');
    await fireEvent.input(field, { target: { value: 'secret' } });
    await fireEvent.keyDown(field, { key: 'Enter' });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(passwords.submit).toHaveBeenCalledExactlyOnceWith('secret');
    expect(passwords.cancel).toHaveBeenCalledOnce();
  });
});
