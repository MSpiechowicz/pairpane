/** Something a lane is asked about: a file, or a folder, under the key that names it. */
export interface Keyed {
  key: string;
}

/**
 * How long a lane waits after the backend said it is busy: the first time, then twice as long each time it says so
 * again, up to {@link BUSY_WAIT_MAX_MS}; an answer that is not busy starts over.
 */
const BUSY_WAIT_MS = 300;
const BUSY_WAIT_MAX_MS = 5_000;
/** How many busy answers in a row make a lane stop asking (a mount that hangs keeps the backend's threads for good). */
export const MAX_BUSY_IN_A_ROW = 8;

/**
 * One kind of request: how many may be out at once, the wait after the backend said it is busy, the files that wait
 * for their time (a passing failure), and whether it has given up for the files it was asked for.
 */
export class Lane {
  /** The keys of the files being asked about. */
  readonly flying = new Set<string>();
  /** How many requests are out (one may be about several files). */
  #out = 0;
  readonly #parallel: number;
  #holdUntil = 0;
  #wait = BUSY_WAIT_MS;
  #busyRun = 0;
  /** Set once the lane gave up: the keys that were queued then. It asks again when the pane wants others. */
  #stoppedOn: ReadonlySet<string> | undefined;
  /** Files not to be asked about before a time, by key. */
  readonly #notBefore = new Map<string, number>();

  constructor(parallel: number) {
    this.#parallel = parallel;
  }

  get free(): boolean {
    return this.#out < this.#parallel && this.#stoppedOn === undefined;
  }

  /**
   * The items of `queue` that may be asked about now (not out, not waiting for their time), and when the first of the
   * others may be.
   */
  due<T extends Keyed>(queue: readonly T[], now: number): { due: T[]; next: number | undefined } {
    const due: T[] = [];
    let next: number | undefined;

    for (const item of queue) {
      const after = this.notBefore(item.key, now);
      if (this.flying.has(item.key)) {
        continue;
      }

      if (after === undefined) {
        due.push(item);
      } else {
        next = Math.min(next ?? after, after);
      }
    }

    return { due, next };
  }

  /**
   * Sends requests while the lane is free and something of `queue` may be asked about now: `send` is given what may go,
   * the first first, and starts one request (it must call {@link start} at once). Returns when to look again if the
   * lane is waiting (for the backend, or for the time of an item) with work to do.
   */
  drain<T extends Keyed>(queue: readonly T[], now: number, send: (due: T[]) => void): number | undefined {
    let next: number | undefined;

    while (this.free) {
      const queued = this.due(queue, now);
      next = queued.next;
      if (queued.due.length === 0) {
        return next;
      }

      if (this.waits(now) > 0) {
        return Math.min(next ?? Infinity, now + this.waits(now));
      }

      send(queued.due);
    }

    return next;
  }

  /** A request about the files `targets` goes out. */
  start(targets: readonly Keyed[]) {
    this.#out += 1;
    targets.forEach((target) => this.flying.add(target.key));
  }

  /** The request about `targets` is answered. */
  finish(targets: readonly Keyed[]) {
    this.#out -= 1;
    targets.forEach((target) => this.flying.delete(target.key));
  }

  /** How long the lane still waits, `0` when requests may go. */
  waits(now: number): number {
    return Math.max(0, this.#holdUntil - now);
  }

  /** The backend is full: nothing new for a while, a longer one each time, and none at all after too many in a row. */
  busy(now: number, queue: readonly Keyed[]) {
    this.#holdUntil = Math.max(this.#holdUntil, now + this.#wait);
    this.#wait = Math.min(this.#wait * 2, BUSY_WAIT_MAX_MS);
    this.#busyRun += 1;

    if (this.#busyRun >= MAX_BUSY_IN_A_ROW) {
      this.#stoppedOn = new Set(queue.map((target) => target.key));
    }
  }

  /**
   * An answer that is not busy: the next wait starts over, unless the lane is still waiting (that answer is of a request
   * sent before the backend said it is busy, and says nothing of how full it is now).
   */
  answered(now: number) {
    if (now >= this.#holdUntil) {
      this.#wait = BUSY_WAIT_MS;
      this.#busyRun = 0;
    }
  }

  /** The file is not asked about again before `time`. */
  delay(key: string, time: number) {
    this.#notBefore.set(key, time);
  }

  /** When the file may be asked about again; `undefined` when it may be now. */
  notBefore(key: string, now: number): number | undefined {
    const after = this.#notBefore.get(key);
    return after !== undefined && after > now ? after : undefined;
  }

  /**
   * The pane wants `queue` now. A wait that is over, or of a file no longer wanted, is forgotten (the file is asked
   * about again if it comes back). A lane that gave up asks again once something is wanted that it was not asked for then.
   */
  retarget(queue: readonly Keyed[], now: number) {
    const keys = new Set(queue.map((target) => target.key));
    for (const [key, after] of this.#notBefore) {
      if (after <= now || !keys.has(key)) {
        this.#notBefore.delete(key);
      }
    }

    if (this.#stoppedOn !== undefined && queue.some((target) => !this.#stoppedOn?.has(target.key))) {
      this.#stoppedOn = undefined;
      this.#busyRun = 0;
    }
  }
}

/** A look at the queues set for a later time (a lane that waits, a value that expires); one that is set for later is moved up. */
export class Wake {
  #timer: ReturnType<typeof setTimeout> | undefined;
  #until = 0;
  readonly #look: () => void;

  constructor(look: () => void) {
    this.#look = look;
  }

  /** Looks at the queues again at `time`, unless a look is set for before that. */
  at(time: number, now: number) {
    if (this.#timer !== undefined) {
      if (this.#until <= time) {
        return;
      }

      clearTimeout(this.#timer);
    }

    this.#until = time;
    this.#timer = setTimeout(() => {
      this.#timer = undefined;
      this.#look();
    }, time - now);
  }
}
