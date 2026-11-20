import { describe, expect, it } from 'vitest';

import { selectTransport } from './client';
import { IpcFailure } from './transport';

describe('selectTransport', () => {
  it('rejects every call with a clear error when no backend is available', async () => {
    const transport = selectTransport(false);
    const attempt = transport.invoke('app_info', undefined);

    await expect(attempt).rejects.toBeInstanceOf(IpcFailure);
    await expect(attempt).rejects.toThrow(/desktop runtime is not available/);
  });
});
