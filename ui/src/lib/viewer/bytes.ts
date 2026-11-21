import { retryBusy } from './busy';
import type { ViewSession } from './session.svelte';

/** The most `view_bytes` answers in one call. */
export const CHUNK = 4 * 1024 * 1024;

/** The biggest image the viewer reads whole. */
export const IMAGE_CAP = 64 * 1024 * 1024;

/** The file is bigger than the viewer is willing to hold in memory. */
export class TooBig extends Error {
  constructor() {
    super('The file is too big to show here');
    this.name = 'TooBig';
  }
}

/** One `view_bytes` answer: `length` bytes from `offset` (fewer at the end of the file), asked again while busy. */
export async function readBytes(
  session: ViewSession,
  offset: number,
  length: number,
  signal?: AbortSignal,
): Promise<ArrayBuffer> {
  const id = session.id;
  if (id === undefined) {
    return new ArrayBuffer(0);
  }

  return retryBusy(() => session.ipc.invoke('view_bytes', { session: id, offset, length }), signal);
}

/**
 * The bytes from `begin` up to `end` (or the end of the file), one `view_bytes` answer of at most 4 MiB at a time,
 * each with the offset it starts at. It stops when the signal aborts (by throwing the signal's reason) and at the end
 * of the file, where an answer comes up short.
 */
export async function* chunks(
  session: ViewSession,
  begin: number,
  end: number,
  signal?: AbortSignal,
): AsyncGenerator<{ offset: number; bytes: Uint8Array }> {
  let offset = begin;

  while (offset < end && session.id !== undefined) {
    signal?.throwIfAborted();

    const bytes = new Uint8Array(await readBytes(session, offset, Math.min(CHUNK, end - offset), signal));
    signal?.throwIfAborted();

    if (bytes.length === 0) {
      return;
    }

    yield { offset, bytes };
    offset += bytes.length;
  }
}

/** The whole file, when it is at most `cap` bytes long; otherwise {@link TooBig}, before a byte is read. */
export async function readWhole(session: ViewSession, cap: number, signal?: AbortSignal): Promise<Uint8Array> {
  const size = session.size;
  if (size > cap) {
    throw new TooBig();
  }

  const whole = new Uint8Array(size);
  for await (const { offset, bytes } of chunks(session, 0, size, signal)) {
    whole.set(bytes.subarray(0, size - offset), offset);
  }

  return whole;
}
