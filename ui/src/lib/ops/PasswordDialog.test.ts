import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import PasswordDialog from './PasswordDialog.svelte';

function open(extra: Partial<Parameters<typeof PasswordDialog>[1]> = {}) {
  const onSubmit = vi.fn();
  const onCancel = vi.fn();
  render(PasswordDialog, { title: 'Password for “host”', message: 'The server asks for a password.', busy: false, onSubmit, onCancel, ...extra });
  return { onSubmit, onCancel, field: () => screen.getByLabelText(extra.label ?? 'Password') as HTMLInputElement };
}

describe('PasswordDialog', () => {
  it('shows what it is asked to: the title, the message, and a field that hides what is typed', () => {
    const { field } = open();

    expect(screen.getByRole('dialog', { name: 'Password for “host”' })).toBeInTheDocument();
    expect(screen.getByText('The server asks for a password.')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(field().type).toBe('password');
  });

  it('is an alert when the last try was wrong, and shows another failure too', () => {
    open({ alert: true, message: 'The password is wrong. Try again.', failure: 'Connection closed.' });

    expect(screen.getByText('The password is wrong. Try again.')).toHaveAttribute('role', 'alert');
    expect(screen.getByText('Connection closed.')).toBeInTheDocument();
  });

  it('sends what was typed on Enter and empties the field at once; an empty field sends nothing', async () => {
    const { field, onSubmit } = open();

    await fireEvent.keyDown(field(), { key: 'Enter' });
    expect(onSubmit).not.toHaveBeenCalled();

    await fireEvent.input(field(), { target: { value: 'hunter2' } });
    await fireEvent.keyDown(field(), { key: 'Enter' });

    expect(onSubmit).toHaveBeenCalledExactlyOnceWith('hunter2');
    expect(field().value).toBe('');
  });

  it('sends nothing while what was typed is being checked', async () => {
    const { field, onSubmit } = open({ busy: true });

    await fireEvent.input(field(), { target: { value: 'x' } });
    await fireEvent.keyDown(field(), { key: 'Enter' });

    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('turns the field off while it is checked, and empties it when the answer is in', async () => {
    const { rerender } = render(PasswordDialog, { title: 't', message: 'm', busy: false, onSubmit: vi.fn(), onCancel: vi.fn() });
    const field = screen.getByLabelText('Password') as HTMLInputElement;
    await fireEvent.input(field, { target: { value: 'half' } });

    await rerender({ busy: true });
    expect(field).toBeDisabled();
    await fireEvent.input(field, { target: { value: 'typed-while-busy' } });

    await rerender({ busy: false });
    expect(field).not.toBeDisabled();
    expect(field.value).toBe('');
  });

  it('names the field as it is told (a passphrase) and gives up with Cancel', async () => {
    const { field, onCancel } = open({ label: 'Passphrase' });

    expect(field()).toBeInTheDocument();
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onCancel).toHaveBeenCalledOnce();
  });
});
