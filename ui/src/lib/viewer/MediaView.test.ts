import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));

import type { ViewKind } from '$lib/ipc/bindings/ViewKind';

import { FakeFile, openedSession } from '../../../tests/view';
import MediaView from './MediaView.svelte';

async function open(kind: ViewKind, token: () => string | Promise<string> = () => 'tok123') {
  const world = await openedSession(new FakeFile('media'), () => kind, { view_media: token });
  const view = render(MediaView, { session: world.session });

  return { ...world, view };
}

describe('MediaView', () => {
  it('plays audio from the custom scheme, at the address the token makes', async () => {
    const { sent, view } = await open({ kind: 'audio', mime: 'audio/mpeg' });

    await vi.waitFor(() => expect(view.container.querySelector('audio')).not.toBeNull());

    expect(sent('view_media')).toEqual([{ session: 1 }]);
    const player = view.container.querySelector('audio');
    expect(player).toHaveAttribute('src', 'pairpane-media://localhost/tok123');
    expect(player).toHaveAttribute('controls');
    expect(view.container.querySelector('video')).toBeNull();
  });

  it('plays video in a video element', async () => {
    const { view } = await open({ kind: 'video', mime: 'video/mp4' });

    await vi.waitFor(() => expect(view.container.querySelector('video')).toHaveAttribute('src', 'pairpane-media://localhost/tok123'));
    expect(view.container.querySelector('audio')).toBeNull();
  });

  it('says so when the webview cannot play the file', async () => {
    const { view } = await open({ kind: 'video', mime: 'video/mp4' });
    await vi.waitFor(() => expect(view.container.querySelector('video')).not.toBeNull());

    await fireEvent.error(view.container.querySelector('video') as HTMLVideoElement);

    expect(screen.getByRole('alert')).toHaveTextContent('This file cannot be played here');
  });

  it('shows the backend`s words when there is no address', async () => {
    await open({ kind: 'audio', mime: 'audio/mpeg' }, () =>
      Promise.reject({ code: 'unsupported', message: 'Too many files are playing' }),
    );

    expect(await screen.findByRole('alert')).toHaveTextContent('Too many files are playing');
  });

  it('lets go of the address when the view goes', async () => {
    const { view } = await open({ kind: 'audio', mime: 'audio/mpeg' });
    await vi.waitFor(() => expect(view.container.querySelector('audio')).not.toBeNull());
    const player = view.container.querySelector('audio') as HTMLAudioElement;

    view.unmount();

    expect(player).not.toHaveAttribute('src');
  });
});
