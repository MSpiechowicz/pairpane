import { fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { offered } from '../../../tests/devices';
import PairPanel from './PairPanel.svelte';

afterEach(() => vi.restoreAllMocks());

function show(changes = {}, remaining = 65_000) {
  const onCancel = vi.fn();
  render(PairPanel, { offer: offered(changes), remaining, onCancel });

  return { onCancel };
}

describe('the code that is offered', () => {
  it('shows the code, the time that is left and the address to give', () => {
    show();

    expect(screen.getByLabelText('Pairing code')).toHaveTextContent('482913');
    expect(screen.getByRole('timer', { name: 'Time left for the code' })).toHaveTextContent('Expires in 1:05');
    expect(screen.getByLabelText('Address')).toHaveValue('node-target-text');
    expect(screen.getByLabelText('Address')).toHaveAttribute('readonly');
  });

  it('draws the QR code as an image whose path is the one it is given', () => {
    show();

    const image = screen.getByRole('img', { name: "QR code of this device's address" });
    expect(image.querySelector('path')).toHaveAttribute('d', 'M0 0h1v1h-1z');
    expect(image).toHaveAttribute('viewBox', '-4 -4 29 29');
  });

  it('has no image when the address does not fit into a QR code', () => {
    show({ qr: null });

    expect(screen.queryByRole('img')).not.toBeInTheDocument();
  });

  it('copies the address to the clipboard', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } });
    show();

    await fireEvent.click(screen.getByRole('button', { name: 'Copy address' }));

    expect(writeText).toHaveBeenCalledWith('node-target-text');
    expect(await screen.findByText('Copied.')).toBeInTheDocument();
    vi.unstubAllGlobals();
  });

  it('says so when the clipboard is not available', async () => {
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) } });
    show();

    await fireEvent.click(screen.getByRole('button', { name: 'Copy address' }));

    expect(await screen.findByText(/copy it by hand/)).toBeInTheDocument();
    vi.unstubAllGlobals();
  });

  it('Cancel withdraws it', async () => {
    const { onCancel } = show();

    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});
