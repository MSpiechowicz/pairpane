import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { BUSY_BACKOFF_MS, retryBusy } from './busy';

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

const busy = { code: 'busy', message: 'the viewer is busy' };

describe('retryBusy', () => {
  it('answers at once when the read goes through', async () => {
    const read = vi.fn().mockResolvedValue(7);

    await expect(retryBusy(read)).resolves.toBe(7);
    expect(read).toHaveBeenCalledTimes(1);
  });

  it('tries again after each wait of the backoff while the backend is busy', async () => {
    const read = vi.fn().mockRejectedValueOnce(busy).mockRejectedValueOnce(busy).mockResolvedValue('bytes');
    const answer = retryBusy(read);

    await vi.advanceTimersByTimeAsync(BUSY_BACKOFF_MS[0] - 1);
    expect(read).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(read).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(BUSY_BACKOFF_MS[1]);

    await expect(answer).resolves.toBe('bytes');
    expect(read).toHaveBeenCalledTimes(3);
  });

  it('waits as long as a backoff of its own says', async () => {
    const read = vi.fn().mockRejectedValue(busy);
    const answer = retryBusy(read, undefined, [1000, 3000]);
    const failed = expect(answer).rejects.toMatchObject(busy);

    await vi.advanceTimersByTimeAsync(3999);
    expect(read).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1);

    await failed;
    expect(read).toHaveBeenCalledTimes(3);
  });

  it('gives up with the backend`s refusal once the waits are spent', async () => {
    const read = vi.fn().mockRejectedValue(busy);
    const answer = retryBusy(read);
    const failed = expect(answer).rejects.toMatchObject(busy);

    await vi.advanceTimersByTimeAsync(BUSY_BACKOFF_MS.reduce((sum, wait) => sum + wait, 0));

    await failed;
    expect(read).toHaveBeenCalledTimes(BUSY_BACKOFF_MS.length + 1);
  });

  it('does not retry a failure that is not busy', async () => {
    const read = vi.fn().mockRejectedValue({ code: 'io', message: 'disk gone' });

    await expect(retryBusy(read)).rejects.toMatchObject({ code: 'io' });
    expect(read).toHaveBeenCalledTimes(1);
  });

  it('stops waiting when the signal aborts', async () => {
    const controller = new AbortController();
    const read = vi.fn().mockRejectedValue(busy);
    const answer = retryBusy(read, controller.signal);

    controller.abort();
    const failed = expect(answer).rejects.toBe(controller.signal.reason);
    await vi.advanceTimersByTimeAsync(BUSY_BACKOFF_MS[0]);

    await failed;
    expect(read).toHaveBeenCalledTimes(1);
  });
});
