import { render, screen } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import App from './App.svelte';

describe('App', () => {
  beforeEach(() => invoke.mockReset());

  it('shows the app name and the version reported by the backend', async () => {
    invoke.mockResolvedValueOnce({ name: 'Pairpane', version: '0.1.0' });

    render(App);

    expect(screen.getByRole('heading', { name: 'Pairpane' })).toBeInTheDocument();
    expect(await screen.findByText('v0.1.0')).toBeInTheDocument();
  });

  it('shows a failure message when the backend call is rejected', async () => {
    invoke.mockRejectedValueOnce(new Error('The desktop runtime is not available.'));

    render(App);

    expect(await screen.findByRole('alert')).toHaveTextContent('The desktop runtime is not available.');
    expect(screen.queryByText(/^v\d/)).not.toBeInTheDocument();
  });
});
