import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import Splash from './Splash.svelte';
import { STORAGE_KEY } from './splashGate';

const TAGLINE = /One file manager/;
const originalAnimate = Element.prototype.animate;

/** The page's motion preference (the test environment defaults to reduced motion). */
function prefersReducedMotion(reduced: boolean) {
  vi.spyOn(window, 'matchMedia').mockImplementation(
    (query) => ({ matches: reduced && query.includes('reduced-motion'), addEventListener: vi.fn() }) as unknown as MediaQueryList,
  );
}

/** Animations that are over at once, so only the splash's own sequencing takes time. */
function instantAnimations() {
  vi.spyOn(Element.prototype, 'animate').mockImplementation(() => ({ finished: Promise.resolve() }) as unknown as Animation);
}

describe('Splash', () => {
  beforeEach(() => {
    window.localStorage.clear();
    prefersReducedMotion(false);
    instantAnimations();
  });

  afterEach(() => {
    Element.prototype.animate = originalAnimate;
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('plays on the first launch: the name, the tagline and the version, hidden from assistive technology', () => {
    const { container } = render(Splash, { version: '1.2.3' });

    expect(screen.getByText(TAGLINE)).toBeInTheDocument();
    expect(screen.getByText('v1.2.3')).toBeInTheDocument();
    expect(container.querySelector('[aria-hidden="true"]')).toContainElement(screen.getByText(TAGLINE));
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe('1.2.3');
  });

  it('plays after an update and remembers the new version', () => {
    window.localStorage.setItem(STORAGE_KEY, '1.2.2');

    render(Splash, { version: '1.2.3' });

    expect(screen.getByText(TAGLINE)).toBeInTheDocument();
    expect(window.localStorage.getItem(STORAGE_KEY)).toBe('1.2.3');
  });

  it('plays even when the storage throws', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });

    render(Splash, { version: '1.2.3' });

    expect(screen.getByText(TAGLINE)).toBeInTheDocument();
  });

  it('renders nothing for a version that already played', () => {
    window.localStorage.setItem(STORAGE_KEY, '1.2.3');

    render(Splash, { version: '1.2.3' });

    expect(screen.queryByText(TAGLINE)).not.toBeInTheDocument();
    expect(Element.prototype.animate).not.toHaveBeenCalled();
  });

  it('renders nothing under reduced motion, and does not record the version', () => {
    prefersReducedMotion(true);

    render(Splash, { version: '1.2.3' });

    expect(screen.queryByText(TAGLINE)).not.toBeInTheDocument();
    expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull();
  });

  it('is over at once where there is no Web Animations API', () => {
    // @ts-expect-error The method is removed to look like a webview without it.
    Element.prototype.animate = undefined;

    render(Splash, { version: '1.2.3' });

    expect(screen.queryByText(TAGLINE)).not.toBeInTheDocument();
  });

  it('removes itself when the sequence is over and then reports it', async () => {
    vi.useFakeTimers();
    const onEnd = vi.fn();
    render(Splash, { version: '1.2.3', onEnd });
    expect(screen.getByText(TAGLINE)).toBeInTheDocument();

    await vi.advanceTimersByTimeAsync(15_000);

    expect(screen.queryByText(TAGLINE)).not.toBeInTheDocument();
    expect(onEnd).toHaveBeenCalledOnce();
  });

  it('ends even when animations and animation frames stall (a minimised or throttled window)', async () => {
    vi.useFakeTimers();
    vi.spyOn(Element.prototype, 'animate').mockImplementation(
      () => ({ finished: new Promise(() => {}) }) as unknown as Animation,
    );
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 0);
    const reachedApp = vi.fn();
    window.addEventListener('keydown', reachedApp);
    const onEnd = vi.fn();
    render(Splash, { version: '1.2.3', onEnd });

    await vi.advanceTimersByTimeAsync(15_000);

    expect(screen.queryByText(TAGLINE)).not.toBeInTheDocument();
    expect(onEnd).toHaveBeenCalledOnce();
    await fireEvent.keyDown(document.body, { key: 'a' });
    expect(reachedApp).toHaveBeenCalled(); // the keyboard is the app's again
    window.removeEventListener('keydown', reachedApp);
  });

  it('ends early on a click, which does not reach the app underneath', async () => {
    const reachedApp = vi.fn();
    window.addEventListener('click', reachedApp);
    const onEnd = vi.fn();
    render(Splash, { version: '1.2.3', onEnd });

    await fireEvent.click(document.body);

    await waitFor(() => expect(screen.queryByText(TAGLINE)).not.toBeInTheDocument());
    expect(onEnd).toHaveBeenCalledOnce();
    expect(reachedApp).not.toHaveBeenCalled();
    window.removeEventListener('click', reachedApp);
  });

  it('ends early on a key press, which does not reach the app underneath', async () => {
    const reachedApp = vi.fn();
    window.addEventListener('keydown', reachedApp);
    render(Splash, { version: '1.2.3' });

    const notPrevented = await fireEvent.keyDown(document.body, { key: 'Tab' });

    expect(notPrevented).toBe(false);
    await waitFor(() => expect(screen.queryByText(TAGLINE)).not.toBeInTheDocument());
    expect(reachedApp).not.toHaveBeenCalled();
    window.removeEventListener('keydown', reachedApp);
  });

  it('lets input through to the app once it is gone', async () => {
    const reachedApp = vi.fn();
    window.addEventListener('keydown', reachedApp);
    render(Splash, { version: '1.2.3' });
    await fireEvent.keyDown(document.body, { key: 'Tab' });
    await waitFor(() => expect(screen.queryByText(TAGLINE)).not.toBeInTheDocument());

    await fireEvent.keyDown(document.body, { key: 'a' });

    expect(reachedApp).toHaveBeenCalledOnce();
    window.removeEventListener('keydown', reachedApp);
  });
});
