import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { OpsEvent } from '$lib/ipc/bindings/OpsEvent';
import { IpcFailure } from '$lib/ipc/transport';

import { conflict, job, opsBackend } from '../../../tests/ops';
import { FADE_MS, LINGER_MS, Transfers } from './transfers.svelte';

/** A store that has subscribed, with the backend to drive it. */
async function subscribed(answers: Parameters<typeof opsBackend>[0] = {}) {
  const backend = opsBackend(answers);
  const transfers = new Transfers(backend.ipc);
  await transfers.subscribe();
  return { ...backend, transfers };
}

const done = (id: number, extra: Partial<Extract<OpsEvent, { kind: 'ended' }>> = {}): OpsEvent => ({
  kind: 'ended',
  id,
  outcome: { kind: 'done' },
  skipped: 0,
  errors: [],
  moreErrors: 0,
  ...extra,
});

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('Transfers', () => {
  it('subscribes once and starts from the snapshot: the jobs and the open questions', async () => {
    const { transfers, send, calls } = await subscribed();
    expect(calls('ops_subscribe')).toHaveLength(1);

    send({ kind: 'snapshot', jobs: [job(1), job(2, { label: 'b.pdf' })], questions: [conflict(2, 3)] });

    expect(transfers.jobs.map((row) => row.label)).toEqual(['a.pdf', 'b.pdf']);
    expect(transfers.question?.info).toMatchObject({ job: 2, question: 3 });
  });

  it('is running from the first job until the last one has ended, a paused one included', async () => {
    const { transfers, send } = await subscribed();
    expect(transfers.running).toBe(false);

    send({ kind: 'added', job: job(1) }, { kind: 'added', job: job(2) }, { kind: 'paused', id: 2, paused: true });
    expect(transfers.running).toBe(true);

    send(done(1));
    expect(transfers.running).toBe(true);

    send(done(2));
    expect(transfers.running).toBe(false);
    expect(transfers.jobs).toHaveLength(2); // the rows stay a while: what has ended does not count
  });

  it('a later snapshot (a reloaded page) replaces what was there', async () => {
    const { transfers, send } = await subscribed();
    send({ kind: 'added', job: job(1) }, { kind: 'conflict', conflict: conflict(1) });

    send({ kind: 'snapshot', jobs: [job(5)], questions: [] });

    expect(transfers.jobs.map((row) => row.id)).toEqual([5]);
    expect(transfers.question).toBeUndefined();
  });

  it('follows a job: added, phase, paused and progress', async () => {
    const { transfers, send } = await subscribed();

    send({ kind: 'added', job: job(1, { phase: 'queued' }) });
    send({ kind: 'phase', id: 1, phase: 'copying' });
    send({ kind: 'paused', id: 1, paused: true });
    send({
      kind: 'progress',
      id: 1,
      doneBytes: 50,
      totalBytes: 200,
      doneFiles: 1,
      totalFiles: 2,
      current: 'a.pdf',
      bytesPerSec: 10,
      etaSecs: 15,
    });

    expect(transfers.jobs[0]).toMatchObject({
      phase: 'copying',
      paused: true,
      doneBytes: 50,
      totalBytes: 200,
      current: 'a.pdf',
      bytesPerSec: 10,
      etaSecs: 15,
    });
  });

  it('events of a job it does not know are ignored', async () => {
    const { transfers, send } = await subscribed();

    send({ kind: 'phase', id: 9, phase: 'copying' }, done(9));

    expect(transfers.jobs).toEqual([]);
  });

  it('the chip`s numbers: every job counted, the percent from the bytes of the running ones', async () => {
    const { transfers, send } = await subscribed();
    expect(transfers.overall).toEqual({ count: 0, percent: undefined });

    send({ kind: 'added', job: job(1, { doneBytes: 25, totalBytes: 100 }) });
    send({ kind: 'added', job: job(2, { doneBytes: 65, totalBytes: 100 }) });
    send({ kind: 'added', job: job(3) });
    expect(transfers.overall).toEqual({ count: 3, percent: 45 });

    send(done(2));
    expect(transfers.overall).toEqual({ count: 3, percent: 25 });
  });

  describe('when a job ends', () => {
    it('tells the owner which job, and how it ended, once its row shows the result; an unknown job is not told', async () => {
      const backend = opsBackend();
      const heard: unknown[] = [];
      const transfers = new Transfers(backend.ipc, (ended, result) =>
        heard.push({ id: ended.id, kind: ended.kind, to: ended.to, outcome: result.outcome, shown: transfers.jobs[0].result !== undefined }),
      );
      await transfers.subscribe();

      backend.send({ kind: 'added', job: job(1, { kind: 'addToArchive', to: '/work/a.zip' }) }, done(1), done(7));

      expect(heard).toEqual([{ id: 1, kind: 'addToArchive', to: '/work/a.zip', outcome: { kind: 'done' }, shown: true }]);
    });

    it('a clean one stays for a while as finished, then fades and goes', async () => {
      const { transfers, send } = await subscribed();
      send({ kind: 'added', job: job(1) }, done(1));
      expect(transfers.jobs[0].result?.outcome).toEqual({ kind: 'done' });

      vi.advanceTimersByTime(LINGER_MS - 1);
      expect(transfers.jobs[0].leaving).toBeUndefined();

      vi.advanceTimersByTime(1);
      expect(transfers.jobs[0].leaving).toBe(true);

      vi.advanceTimersByTime(FADE_MS);
      expect(transfers.jobs).toEqual([]);
    });

    it('a failed one stays, with its error, until it is dismissed', async () => {
      const { transfers, send } = await subscribed();
      const error = { code: 'io' as const, message: 'No space left' };
      send({ kind: 'added', job: job(1) }, done(1, { outcome: { kind: 'failed', error } }));

      vi.advanceTimersByTime(LINGER_MS + FADE_MS + 1000);
      expect(transfers.jobs).toHaveLength(1);

      transfers.dismiss(1);
      expect(transfers.jobs).toEqual([]);
    });

    it('a job that finished but lost some items stays too', async () => {
      const { transfers, send } = await subscribed();
      const errors = [{ path: 'x/y.txt', error: { code: 'io' as const, message: 'Permission denied' } }];
      send({ kind: 'added', job: job(1) }, done(1, { errors, moreErrors: 4 }));

      vi.advanceTimersByTime(LINGER_MS + FADE_MS + 1000);

      expect(transfers.jobs[0].result).toMatchObject({ errors, moreErrors: 4 });
    });

    it('a running job cannot be dismissed, and a late progress does not bring a finished one back to life', async () => {
      const { transfers, send } = await subscribed();
      send({ kind: 'added', job: job(1) });
      transfers.dismiss(1);
      expect(transfers.jobs).toHaveLength(1);

      send(done(1), { kind: 'phase', id: 1, phase: 'copying' });
      expect(transfers.jobs[0].result).toBeDefined();
    });

    it('takes the job`s open questions with it', async () => {
      const { transfers, send } = await subscribed();
      send({ kind: 'added', job: job(1) }, { kind: 'conflict', conflict: conflict(1) }, done(1));

      expect(transfers.question).toBeUndefined();
    });
  });

  describe('control', () => {
    it('sends the action for the job', async () => {
      const { transfers, calls } = await subscribed();

      await transfers.control(4, 'pause');

      expect(calls('ops_control')).toEqual([{ job: 4, action: 'pause' }]);
    });

    it('a job that is gone is not an error', async () => {
      const gone = async () => {
        throw new IpcFailure({ code: 'notFound', message: 'no such job' });
      };
      const { transfers } = await subscribed({ ops_control: gone });

      await expect(transfers.control(4, 'cancel')).resolves.toBeUndefined();
    });
  });

  describe('questions', () => {
    it('queue, and the oldest is asked first, one at a time', async () => {
      const { transfers, send } = await subscribed();
      send({ kind: 'added', job: job(1) }, { kind: 'added', job: job(2) });
      send({ kind: 'conflict', conflict: conflict(2, 1) }, { kind: 'conflict', conflict: conflict(1, 1) });
      send({ kind: 'conflict', conflict: conflict(2, 1) }); // the same one again

      expect(transfers.question?.info.job).toBe(2);

      await transfers.answer(transfers.question!, 'skip', false);
      expect(transfers.question?.info.job).toBe(1);

      await transfers.answer(transfers.question!, 'skip', false);
      expect(transfers.question).toBeUndefined();
    });

    it('a conflict of an unknown or an ended job is not asked', async () => {
      const { transfers, send } = await subscribed();

      send({ kind: 'conflict', conflict: conflict(9) }); // never added
      expect(transfers.question).toBeUndefined();

      send({ kind: 'added', job: job(1) }, done(1), { kind: 'conflict', conflict: conflict(1) }); // a late one
      expect(transfers.question).toBeUndefined();
    });

    it('a final answer goes to the backend with the question`s number and the apply-to-all flag', async () => {
      const { transfers, send, calls } = await subscribed();
      send({ kind: 'added', job: job(7) }, { kind: 'conflict', conflict: conflict(7, 3) });

      await transfers.answer(transfers.question!, 'overwriteOlder', true);

      expect(calls('ops_answer')).toEqual([{ job: 7, question: 3, answer: 'overwriteOlder', all: true }]);
    });

    it('Compare is not final: the question stays and shows what the comparison found', async () => {
      const { transfers, send, calls } = await subscribed();
      send({ kind: 'added', job: job(1) }, { kind: 'conflict', conflict: conflict(1, 2) });

      const asking = transfers.answer(transfers.question!, 'compare', false);
      expect(transfers.question?.compare).toBe('running');
      await asking;
      expect(calls('ops_answer')).toEqual([{ job: 1, question: 2, answer: 'compare', all: false }]);

      send({ kind: 'compared', id: 1, question: 2, identical: false });
      expect(transfers.question).toMatchObject({ compare: 'different', info: { question: 2 } });

      send({ kind: 'compared', id: 1, question: 9, identical: true }); // another question's result
      expect(transfers.question?.compare).toBe('different');
    });

    it('a compare the backend refused can be tried again', async () => {
      const busy = async () => {
        throw new IpcFailure({ code: 'busy', message: 'busy' });
      };
      const { transfers, send } = await subscribed({ ops_answer: busy });
      send({ kind: 'added', job: job(1) }, { kind: 'conflict', conflict: conflict(1) });

      await transfers.answer(transfers.question!, 'compare', false);

      expect(transfers.question?.compare).toBeUndefined();
    });

    it('an answer that is stale (refused as invalid or unknown) drops the question for good', async () => {
      for (const code of ['invalidInput', 'notFound'] as const) {
        const stale = async () => {
          throw new IpcFailure({ code, message: 'stale' });
        };
        const { transfers, send } = await subscribed({ ops_answer: stale });
        send({ kind: 'added', job: job(1) }, { kind: 'conflict', conflict: conflict(1) });

        await transfers.answer(transfers.question!, 'skip', false);

        expect(transfers.question).toBeUndefined();
      }
    });

    it('an answer the backend could not take for another reason brings the question back', async () => {
      const failing = async () => {
        throw new IpcFailure({ code: 'internal', message: 'oops' });
      };
      const { transfers, send } = await subscribed({ ops_answer: failing });
      send({ kind: 'added', job: job(1) }, { kind: 'conflict', conflict: conflict(1, 4) });

      await transfers.answer(transfers.question!, 'overwrite', false);

      expect(transfers.question?.info.question).toBe(4);
    });

    it('cancelling the job drops its questions at once and asks the backend to cancel', async () => {
      const { transfers, send, calls } = await subscribed();
      send({ kind: 'added', job: job(1) }, { kind: 'added', job: job(2) });
      send({ kind: 'conflict', conflict: conflict(1) }, { kind: 'conflict', conflict: conflict(2) });

      await transfers.cancelJob(1);

      expect(transfers.question?.info.job).toBe(2);
      expect(calls('ops_control')).toEqual([{ job: 1, action: 'cancel' }]);
    });
  });
});
