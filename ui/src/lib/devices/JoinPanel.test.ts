import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

import type { FoundDevice } from '$lib/ipc/bindings/FoundDevice';

import { ONE, TWO } from '../../../tests/devices';
import JoinPanel from './JoinPanel.svelte';

function show(found: FoundDevice[] = [], busy = false) {
  const onJoin = vi.fn();
  const onCancel = vi.fn();
  render(JoinPanel, { found, busy, onJoin, onCancel });

  return { onJoin, onCancel };
}

const join = () => screen.getByRole('button', { name: /^(Join|Connecting…)$/ });

describe('the devices that are found', () => {
  it('lists them with their short id, "Unnamed" for a nameless one, and a badge on a paired one', () => {
    show([
      { nodeId: ONE, name: 'Phone', paired: false },
      { nodeId: TWO, name: null, paired: true },
    ]);

    const [phone, nameless] = within(screen.getByRole('list', { name: 'Devices found on the network' })).getAllByRole('listitem');
    expect(phone).toHaveTextContent('Phone');
    expect(phone).toHaveTextContent('a1b2c3…c5d6');
    expect(nameless).toHaveTextContent('Unnamed');
    expect(nameless).toHaveTextContent('paired');
    expect(within(nameless).queryByRole('button')).not.toBeInTheDocument();
    expect(screen.getByText('Anyone on the network can announce any name; check the id.')).toBeInTheDocument();
  });

  it('Use puts the id of the device in the target field', async () => {
    show([{ nodeId: ONE, name: 'Phone', paired: false }]);

    await fireEvent.click(screen.getByRole('button', { name: 'Use Phone' }));

    expect(screen.getByLabelText('Address or id')).toHaveValue(ONE);
  });

  it('says it is looking while nothing is found', () => {
    show();

    expect(screen.getByText('Looking for devices on the network…')).toBeInTheDocument();
  });
});

describe('the form', () => {
  it('Join waits for both the target and the code', async () => {
    show();
    expect(join()).toBeDisabled();

    await fireEvent.input(screen.getByLabelText('Address or id'), { target: { value: 'node-x' } });
    expect(join()).toBeDisabled();
    await fireEvent.input(screen.getByLabelText('Code'), { target: { value: '   ' } });
    expect(join()).toBeDisabled();
    await fireEvent.input(screen.getByLabelText('Code'), { target: { value: '123456' } });

    expect(join()).toBeEnabled();
  });

  it('joins with what was typed, on the button and on Enter', async () => {
    const { onJoin } = show();
    await fireEvent.input(screen.getByLabelText('Address or id'), { target: { value: 'node-x' } });
    await fireEvent.input(screen.getByLabelText('Code'), { target: { value: '123456' } });

    await fireEvent.click(join());
    await fireEvent.submit(screen.getByRole('form', { name: 'Join with a code' }));

    expect(onJoin).toHaveBeenCalledTimes(2);
    expect(onJoin).toHaveBeenCalledWith('123456', 'node-x');
  });

  it('is off while connecting', () => {
    show([{ nodeId: ONE, name: 'Phone', paired: false }], true);

    expect(join()).toHaveTextContent('Connecting…');
    expect(join()).toBeDisabled();
    expect(screen.getByLabelText('Code')).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeDisabled();
  });

  it('Cancel closes it', async () => {
    const { onCancel } = show();

    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});
