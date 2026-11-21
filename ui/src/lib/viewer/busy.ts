import { IpcFailure } from '$lib/ipc/transport';

/** The waits before each new try of a read the backend refused as busy; after the last one the refusal is final. */
export const BUSY_BACKOFF_MS = [100, 250, 500];

/**
 * Calls `read`, and again after a short wait when the backend says it is busy (its few read threads are all taken,
 * which a PDF asking for ranges, or a quick view beside it, can cause for a moment). Any other failure, a refusal
 * after the last wait and an abort of the signal during a wait end the call at once.
 */
export async function retryBusy<T>(read: () => Promise<T>, signal?: AbortSignal): Promise<T> {
  for (let attempt = 0; ; attempt += 1) {
    try {
      return await read();
    } catch (error) {
      const wait = BUSY_BACKOFF_MS[attempt];
      if (wait === undefined || IpcFailure.from(error).code !== 'busy') {
        throw error;
      }

      await new Promise((resolve) => setTimeout(resolve, wait));
      signal?.throwIfAborted();
    }
  }
}
