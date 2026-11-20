import { beforeEach, describe, expect, it, vi } from 'vitest';

const { invoke, Channel } = vi.hoisted(() => ({
  invoke: vi.fn(),
  Channel: class {
    onmessage: ((event: unknown) => void) | undefined;
  },
}));
vi.mock('@tauri-apps/api/core', () => ({ invoke, Channel }));

type TauriModule = typeof import('./tauri-transport');
type TransportModule = typeof import('./transport');

let createTauriTransport: TauriModule['createTauriTransport'];
let IpcFailure: TransportModule['IpcFailure'];

describe('createTauriTransport', () => {
  // The module graph may already be loaded, so reload it with the mock in place.
  beforeEach(async () => {
    vi.resetModules();
    ({ createTauriTransport } = await import('./tauri-transport'));
    ({ IpcFailure } = await import('./transport'));
  });

  it('returns the command result unchanged', async () => {
    invoke.mockResolvedValueOnce({ name: 'Pairpane' });

    await expect(createTauriTransport().invoke('app_info', undefined)).resolves.toEqual({ name: 'Pairpane' });
  });

  it('turns a rejected IpcError payload into a typed IpcFailure', async () => {
    invoke.mockRejectedValueOnce({ code: 'forbidden', message: 'path is deny-listed' });

    const failure = await createTauriTransport()
      .invoke('app_info', undefined)
      .catch((error: unknown) => error);

    expect(failure).toBeInstanceOf(IpcFailure);
    expect(failure).toMatchObject({ code: 'forbidden', message: 'path is deny-listed' });
  });

  it('wraps unexpected rejections (strings, errors) as internal failures', async () => {
    invoke.mockRejectedValueOnce('command app_info not found');

    await expect(createTauriTransport().invoke('app_info', undefined)).rejects.toMatchObject({
      code: 'internal',
      message: 'command app_info not found',
    });
  });

  it('turns event-sink arguments into channels that deliver to the sink', async () => {
    invoke.mockResolvedValueOnce(null);
    const received: unknown[] = [];

    await createTauriTransport().invoke('update_install', {
      version: '0.2.0',
      onProgress: (event) => received.push(event),
    });

    const [, args] = invoke.mock.calls.at(-1) as [string, { onProgress: InstanceType<typeof Channel> }];
    expect(args.onProgress).toBeInstanceOf(Channel);

    args.onProgress.onmessage?.({ phase: 'done', downloaded: 1, total: 1 });
    expect(received).toEqual([{ phase: 'done', downloaded: 1, total: 1 }]);
  });
});
