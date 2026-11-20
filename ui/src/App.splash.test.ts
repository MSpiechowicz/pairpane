import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('$lib/ipc/client', () => ({ ipc: { invoke } }));

import { STORAGE_KEY } from '$lib/shell/splashGate';

import App from './App.svelte';

const TAGLINE = /One file manager/;

describe('App with the startup splash', () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockImplementation((name: string) =>
      Promise.resolve(name === 'app_info' ? { name: 'Pairpane', version: '0.1.0' } : null),
    );
    window.localStorage.clear();
    // The splash plays: the user does not prefer reduced motion, and animations are over at once.
    vi.spyOn(window, 'matchMedia').mockImplementation(
      () => ({ matches: false, addEventListener: vi.fn() }) as unknown as MediaQueryList,
    );
    vi.spyOn(Element.prototype, 'animate').mockImplementation(
      () => ({ finished: Promise.resolve() }) as unknown as Animation,
    );
  });

  afterEach(() => vi.restoreAllMocks());

  it('shows no splash on a normal start of a version that already played', () => {
    window.localStorage.setItem(STORAGE_KEY, __APP_VERSION__);

    render(App);

    expect(screen.queryByText(TAGLINE)).not.toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Left pane' })).toBeInTheDocument();
  });

  it('plays it on the first launch, above an app that has already mounted', () => {
    render(App);

    expect(screen.getByText(TAGLINE)).toBeInTheDocument();
    expect(screen.getByText(`v${__APP_VERSION__}`)).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Left pane' })).toBeInTheDocument();
  });

  it('a key press ends it without acting on the app, and the active pane`s list has the focus', async () => {
    render(App);
    const [left] = screen.getAllByRole('listbox');

    expect(await fireEvent.keyDown(left, { key: 'Tab' })).toBe(false);

    await waitFor(() => expect(screen.queryByText(TAGLINE)).not.toBeInTheDocument());
    expect(screen.getByRole('region', { name: 'Left pane' })).toHaveAttribute('aria-current', 'true');
    expect(left).toHaveFocus();
  });

  it('puts the focus on the active list when the splash ends with the focus lost', async () => {
    render(App);
    const [left] = screen.getAllByRole('listbox');
    left.blur();

    await fireEvent.click(document.body);

    await waitFor(() => expect(left).toHaveFocus());
  });
});
