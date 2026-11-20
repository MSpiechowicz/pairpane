import type { Entry } from '$lib/ipc/bindings/Entry';
import type { PaneId } from '$lib/ipc/bindings/PaneId';
import type { SizeEvent } from '$lib/ipc/bindings/SizeEvent';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { formatSize } from './format';

/** A job refused because the previous one has not stopped yet (it waits on a slow disk) is asked again this often… */
export const BUSY_RETRY_MS = 500;
/** …this many times, then dropped (the folder shows no size). */
export const BUSY_RETRIES = 20;

/**
 * The folder sizes a pane has asked for, by folder name: one job at a time, in the order asked (the backend walks
 * the disk, so parallel walks would only slow each other down). Progress arrives as a running byte count, then the
 * final size. Events of a job that is over (the pane moved to another folder) are dropped.
 */
export class FolderSizes {
  readonly #ipc: Transport;
  readonly #pane: PaneId;
  /** The folder the pane shows now. */
  readonly #folder: () => string | undefined;

  /** Final sizes; readers track `#revision`, bumped when one arrives or all are dropped. */
  readonly #known = new Map<string, number>();
  #revision = $state(0);
  #queue: string[] = [];
  /** The folder being measured and the bytes counted so far. */
  #job = $state.raw<{ name: string; bytes: number } | undefined>();
  /** Bumped by every job start and every reset: events of an older epoch are stale. */
  #epoch = 0;

  constructor(ipc: Transport, pane: PaneId, folder: () => string | undefined) {
    this.#ipc = ipc;
    this.#pane = pane;
    this.#folder = folder;
  }

  /** The computed size of the folder `name`, when known. */
  sizeOf(name: string): number | undefined {
    void this.#revision;
    return this.#known.get(name);
  }

  /** What the Size column shows; `undefined` for a folder not measured (yet), which shows the DIR tag. */
  text(entry: Entry): string | undefined {
    if (entry.kind !== 'dir') {
      return formatSize(entry.size);
    }

    const size = this.sizeOf(entry.name);
    if (size !== undefined) {
      return formatSize(size);
    }

    const job = this.#job;
    return job?.name === entry.name ? formatSize(job.bytes) : undefined;
  }

  /** Queues the folder `name`, unless it is known, running or queued already. */
  measure(name: string) {
    if (this.#known.has(name) || this.#job?.name === name || this.#queue.includes(name)) {
      return;
    }

    this.#queue.push(name);
    this.#next();
  }

  /** Forgets every size and stops caring about the running job (the backend stops it when the folder changes). */
  clear() {
    this.#known.clear();
    this.#revision += 1;
    this.#queue = [];
    this.#job = undefined;
    this.#epoch += 1;
  }

  /** After a listing is done: forgets the sizes of folders that are no longer there and drops them from the queue. */
  prune(present: ReadonlySet<string>) {
    this.#queue = this.#queue.filter((name) => present.has(name));

    let dropped = false;
    for (const name of [...this.#known.keys()]) {
      if (!present.has(name)) {
        this.#known.delete(name);
        dropped = true;
      }
    }

    if (dropped) {
      this.#revision += 1;
    }
  }

  /** Whether there is anything to forget. */
  get any(): boolean {
    return this.#known.size > 0 || this.#queue.length > 0;
  }

  #next(retries = 0) {
    const path = this.#folder();
    if (this.#job !== undefined || this.#queue.length === 0 || path === undefined) {
      return;
    }

    const name = this.#queue.shift() as string;
    const epoch = ++this.#epoch;
    this.#job = { name, bytes: 0 };

    const finish = (bytes?: number) => {
      if (epoch !== this.#epoch) {
        return;
      }

      if (bytes !== undefined) {
        this.#known.set(name, bytes);
        this.#revision += 1;
      }

      this.#job = undefined;
      this.#next();
    };

    const onEvent = (event: SizeEvent) => {
      if (epoch !== this.#epoch) {
        return;
      }

      if (event.kind === 'progress') {
        this.#job = { name, bytes: event.bytes };
      } else {
        finish(event.kind === 'done' ? event.bytes : undefined);
      }
    };

    const refused = (error: unknown) => {
      if (IpcFailure.from(error).code !== 'busy' || retries >= BUSY_RETRIES) {
        finish();
        return;
      }

      // The backend is still stopping an earlier job: ask again shortly, unless the pane has moved on meanwhile.
      setTimeout(() => {
        if (epoch === this.#epoch) {
          this.#job = undefined;
          this.#queue.unshift(name);
          this.#next(retries + 1);
        }
      }, BUSY_RETRY_MS);
    };

    this.#ipc.invoke('pane_folder_size', { pane: this.#pane, path, name, onEvent }).catch(refused);
  }
}
