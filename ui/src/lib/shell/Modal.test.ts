import { fireEvent, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import Modal from './Modal.svelte';

const buttons = createRawSnippet(() => ({
  render: () => '<div><button data-autofocus>One</button><button>Two</button><input aria-label="Field" /></div>',
}));

function open(onClose?: () => void) {
  render(Modal, { title: 'Hello', onClose, children: buttons });
}

describe('Modal', () => {
  it('is a labelled dialog, with the focus on what asks for it', () => {
    open();

    expect(screen.getByRole('dialog', { name: 'Hello' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'One' })).toHaveFocus();
  });

  it('Escape closes it, when it can be closed', async () => {
    const onClose = vi.fn();
    open(onClose);

    expect(await fireEvent.keyDown(window, { key: 'Escape' })).toBe(false);

    expect(onClose).toHaveBeenCalledOnce();
  });

  it('without a way to close, Escape does nothing', async () => {
    open();

    expect(await fireEvent.keyDown(window, { key: 'Escape' })).toBe(true);
  });

  it('keeps Tab inside, both ways round', async () => {
    open();
    const [one, two] = [screen.getByRole('button', { name: 'One' }), screen.getByRole('button', { name: 'Two' })];
    const field = screen.getByRole('textbox');

    expect(await fireEvent.keyDown(one, { key: 'Tab' })).toBe(true); // moves on to the next button as usual
    field.focus();
    expect(await fireEvent.keyDown(field, { key: 'Tab' })).toBe(false);
    expect(one).toHaveFocus();

    expect(await fireEvent.keyDown(one, { key: 'Tab', shiftKey: true })).toBe(false);
    expect(field).toHaveFocus();
    expect(two).not.toHaveFocus();
  });

  it('brings a focus that got out (a click on the backdrop) back in', async () => {
    open();
    (document.activeElement as HTMLElement).blur();

    expect(await fireEvent.keyDown(document.body, { key: 'Tab' })).toBe(false);

    expect(screen.getByRole('button', { name: 'One' })).toHaveFocus();
  });

  it('gives the focus back to where it came from when it goes away', () => {
    const before = document.body.appendChild(document.createElement('button'));
    before.focus();

    const { unmount } = render(Modal, { title: 'Hello', children: buttons });
    expect(before).not.toHaveFocus();
    unmount();

    expect(before).toHaveFocus();
  });

  it('only the dialog on top answers the keyboard', async () => {
    const [lower, upper] = [vi.fn(), vi.fn()];
    render(Modal, { title: 'Lower', onClose: lower, children: buttons });
    render(Modal, { title: 'Upper', onClose: upper, children: buttons });

    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(upper).toHaveBeenCalledOnce();
    expect(lower).not.toHaveBeenCalled();
  });

  it('the full variant is the same dialog: labelled, closed by Escape, giving the focus back', async () => {
    const before = document.body.appendChild(document.createElement('button'));
    before.focus();
    const onClose = vi.fn();

    const { unmount } = render(Modal, { title: 'a.pdf', variant: 'full', onClose, children: buttons });
    expect(screen.getByRole('dialog', { name: 'a.pdf' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'One' })).toHaveFocus();

    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledOnce();

    unmount();
    expect(before).toHaveFocus();
  });
});
