import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';

import ConflictHost from '../../../tests/ConflictHost.svelte';
import { conflict, job, opsBackend } from '../../../tests/ops';
import { Transfers } from './transfers.svelte';

/** A store with one job waiting on a question, and the dialog for it. */
async function asked(info = conflict(1, 2)) {
  const backend = opsBackend();
  const transfers = new Transfers(backend.ipc);
  await transfers.subscribe();
  backend.send({ kind: 'added', job: job(info.job) }, { kind: 'conflict', conflict: info });
  render(ConflictHost, { transfers });
  return { ...backend, transfers };
}

const button = (name: string | RegExp) => screen.getByRole('button', { name });

describe('ConflictDialog', () => {
  it('names the file and shows both sides with the newer one marked', async () => {
    await asked();

    expect(screen.getByRole('dialog', { name: '“a.pdf” already exists' })).toBeInTheDocument();
    expect(screen.getByText(/^4\.0 KB/)).toBeInTheDocument();
    expect(screen.getByText(/^2\.0 KB/)).toBeInTheDocument();
    expect(screen.getAllByText('newer')).toHaveLength(1);
    expect(screen.getByText(/^4\.0 KB/).parentElement).toHaveTextContent('newer');
  });

  it('marks nothing when the dates are equal or unknown', async () => {
    const same = conflict(1);
    await asked({ ...same, target: { ...same.target, modified: same.source.modified } });

    expect(screen.queryByText('newer')).not.toBeInTheDocument();
  });

  it('Skip is where the focus starts, and Rename says the name it will use', async () => {
    await asked();

    expect(button('Skip')).toHaveFocus();
    expect(button('Rename → a (2).pdf')).toBeInTheDocument();
  });

  it.each([
    ['Overwrite', 'overwrite'],
    ['Skip', 'skip'],
    ['Rename → a (2).pdf', 'rename'],
    ['Overwrite older', 'overwriteOlder'],
  ])('%s answers %s', async (name, answer) => {
    const { calls } = await asked(conflict(5, 3));

    await fireEvent.click(button(name));

    expect(calls('ops_answer')).toEqual([{ job: 5, question: 3, answer, all: false }]);
  });

  it('Apply to all makes the answer stick for the rest of the job', async () => {
    const { calls } = await asked(conflict(5, 3));
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Apply to all' }));

    await fireEvent.click(button('Overwrite'));

    expect(calls('ops_answer')).toEqual([{ job: 5, question: 3, answer: 'overwrite', all: true }]);
  });

  it('Compare shows the result in place and leaves the choice open', async () => {
    const { calls, send, transfers } = await asked(conflict(5, 3));
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Apply to all' }));

    await fireEvent.click(button('Compare'));
    expect(calls('ops_answer')).toEqual([{ job: 5, question: 3, answer: 'compare', all: false }]);
    expect(await screen.findByRole('button', { name: 'Comparing…' })).toBeDisabled();

    send({ kind: 'compared', id: 5, question: 3, identical: true });
    expect(await screen.findByRole('button', { name: 'Identical' })).toBeDisabled();
    expect(transfers.question).toBeDefined();
    expect(button('Skip')).toBeEnabled();
    expect(button('Overwrite')).toBeEnabled();
  });

  it('Escape skips this one file, even with Apply to all ticked', async () => {
    const { calls } = await asked(conflict(5, 3));
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Apply to all' }));

    await fireEvent.keyDown(window, { key: 'Escape' });

    expect(calls('ops_answer')).toEqual([{ job: 5, question: 3, answer: 'skip', all: false }]);
  });

  it('Cancel job cancels the job instead of answering', async () => {
    const { calls, transfers } = await asked(conflict(5, 3));

    await fireEvent.click(button('Cancel job'));

    expect(calls('ops_control')).toEqual([{ job: 5, action: 'cancel' }]);
    expect(calls('ops_answer')).toEqual([]);
    expect(transfers.question).toBeUndefined();
  });
});
