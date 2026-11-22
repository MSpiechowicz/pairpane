import { untrack } from 'svelte';

import type { RenamePreview } from '$lib/ipc/bindings/RenamePreview';
import type { RenameRow } from '$lib/ipc/bindings/RenameRow';
import type { RenameCounter } from '$lib/ipc/bindings/RenameCounter';
import type { RenameRule } from '$lib/ipc/bindings/RenameRule';
import type { RuleError } from '$lib/ipc/bindings/RuleError';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { blankRule, COUNTER_FIELDS, copyRule, wholeNumber } from './rule';

/** The preview is asked for this long after the last change of the rule, so that typing is one request. */
export const PREVIEW_DELAY_MS = 150;

/**
 * How long to wait before asking again when the backend answered that it is busy (it makes only a few previews at a
 * time), one wait for each new try. When they are all spent the dialog gives up for now and offers to try again.
 */
export const BUSY_RETRY_MS: readonly number[] = [150, 300, 600, 1000, 1000, 1000];

/** A counter field whose text is not a number the rule can hold, and what to tell about it. */
export interface CounterFault {
  key: keyof RenameCounter;
  message: string;
}

/**
 * The rule being written for the items of one folder, and what the backend says it would do: the preview. Every
 * change of the rule asks again; an answer to an earlier question is dropped, so the table never shows a rule the
 * fields no longer hold. The names are only ever computed by the backend.
 */
export class Renamer {
  rule = $state<RenameRule>(blankRule());
  /** The latest answer, kept while the next one is awaited. */
  preview = $state.raw<RenamePreview | undefined>();
  /** Why the backend could not answer (not a fault of the rule). */
  failure = $state<string | undefined>();
  /** The preview is not of the rule as it is now. */
  pending = $state(true);
  /** The backend stayed busy through all the tries: there is no preview of this rule, and `retry` asks again. */
  busy = $state(false);
  /** What each counter field holds as typed; a field not typed in shows the rule's number. */
  counterText = $state<Partial<Record<keyof RenameCounter, string>>>({});

  readonly #ipc: Transport;
  readonly #path: string;
  readonly #names: readonly string[];
  #sequence = 0;
  #latest: RenameRule = blankRule();
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(ipc: Transport, path: string, names: readonly string[]) {
    this.#ipc = ipc;
    this.#path = path;
    this.#names = names;
  }

  /** One row for each item, in the order they were chosen; none while the rule is faulty or nothing was answered yet. */
  get rows(): readonly RenameRow[] {
    return this.preview?.kind === 'rows' ? this.preview.rows : [];
  }

  /** The fault of the rule, with the field it is in. */
  get error(): RuleError | undefined {
    return this.preview?.kind === 'rule' ? this.preview.error : undefined;
  }

  /** The rows that get a new name: unchanged ones are not renamed. */
  get changed(): RenameRow[] {
    return this.rows.filter((row) => row.from !== row.to);
  }

  get problems(): number {
    return this.rows.filter((row) => row.problem !== null).length;
  }

  /** The first counter field whose text the rule does not hold (empty, a fraction, negative, too large). */
  get counterFault(): CounterFault | undefined {
    for (const { key, label, max } of COUNTER_FIELDS) {
      const text = this.counterText[key];
      if (text !== undefined && wholeNumber(text, max) === undefined) {
        return { key, message: `${label} must be a whole number from 0 to ${max.toLocaleString('en-US')}.` };
      }
    }

    return undefined;
  }

  /** The preview is of the rule as the fields hold it, and it can be asked for. */
  get current(): boolean {
    return !this.pending && !this.busy && this.counterFault === undefined;
  }

  /** There is something to rename and nothing in the way: the preview is of this rule, has no fault and no row has a problem. */
  get canApply(): boolean {
    return this.current && this.failure === undefined && this.error === undefined && this.problems === 0 && this.changed.length > 0;
  }

  /** Follows the rule: call while a component is being set up. */
  watch() {
    $effect(() => {
      const rule = $state.snapshot(this.rule) as RenameRule;
      untrack(() => this.#ask(rule));
    });

    $effect(() => () => {
      this.#sequence += 1; // an answer that is still on its way is not wanted
      clearTimeout(this.#timer);
    });
  }

  /** Replaces the rule (a saved one was chosen). */
  load(rule: RenameRule) {
    this.counterText = {};
    this.rule = copyRule(rule);
  }

  /** A counter field was typed in: the rule takes the number, or the field is faulty until it holds one. */
  setCounter(key: keyof RenameCounter, max: number, text: string) {
    this.counterText[key] = text;

    const value = wholeNumber(text, max);
    if (value !== undefined) {
      this.rule.counter[key] = value;
    }
  }

  /** Asks again, after the backend stayed busy. */
  retry() {
    this.#ask(this.#latest);
  }

  #ask(rule: RenameRule) {
    this.#latest = rule;
    this.#sequence += 1;
    this.pending = true;
    this.busy = false;
    clearTimeout(this.#timer);

    const mine = this.#sequence;
    this.#timer = setTimeout(() => void this.#run(rule, mine, 0), PREVIEW_DELAY_MS);
  }

  /** One try at the preview of `rule`; `tries` are the busy answers so far. Nothing is kept of a question that was replaced. */
  async #run(rule: RenameRule, mine: number, tries: number) {
    let preview: RenamePreview | undefined;
    let failure: string | undefined;

    try {
      preview = await this.#ipc.invoke('rename_preview', { path: this.#path, names: [...this.#names], rule });
    } catch (error) {
      const fault = IpcFailure.from(error);
      if (fault.code === 'busy') {
        this.#wait(rule, mine, tries);
        return;
      }

      failure = fault.message;
    }

    if (mine === this.#sequence) {
      this.preview = preview ?? this.preview;
      this.failure = failure;
      this.pending = false;
    }
  }

  /** The backend is busy: tries again after the next wait, and when there is none, stops and says so. */
  #wait(rule: RenameRule, mine: number, tries: number) {
    if (mine !== this.#sequence) {
      return;
    }

    const wait = BUSY_RETRY_MS[tries];
    if (wait === undefined) {
      this.failure = undefined;
      this.busy = true;
      this.pending = false;
      return;
    }

    this.#timer = setTimeout(() => void this.#run(rule, mine, tries + 1), wait);
  }

  /** Queues the rename of the rows that change, to the names the preview showed; a refusal rejects. */
  async apply(): Promise<void> {
    const rows = this.changed;
    await this.#ipc.invoke('ops_task', {
      from: this.#path,
      names: rows.map((row) => row.from),
      task: { kind: 'rename', targets: rows.map((row) => row.to) },
    });
  }
}
