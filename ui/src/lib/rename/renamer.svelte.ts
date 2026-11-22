import { untrack } from 'svelte';

import type { RenamePreview } from '$lib/ipc/bindings/RenamePreview';
import type { RenameRow } from '$lib/ipc/bindings/RenameRow';
import type { RenameRule } from '$lib/ipc/bindings/RenameRule';
import type { RuleError } from '$lib/ipc/bindings/RuleError';
import { IpcFailure, type Transport } from '$lib/ipc/transport';

import { blankRule, copyRule } from './rule';

/** The preview is asked for this long after the last change of the rule, so that typing is one request. */
export const PREVIEW_DELAY_MS = 150;

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

  readonly #ipc: Transport;
  readonly #path: string;
  readonly #names: readonly string[];
  #sequence = 0;
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

  /** There is something to rename and nothing in the way: the preview is of this rule, has no fault and no row has a problem. */
  get canApply(): boolean {
    return !this.pending && this.failure === undefined && this.error === undefined && this.problems === 0 && this.changed.length > 0;
  }

  /** Follows the rule: call while a component is being set up. */
  watch() {
    $effect(() => {
      const rule = $state.snapshot(this.rule) as RenameRule;
      untrack(() => this.#ask(rule));
    });

    $effect(() => () => clearTimeout(this.#timer));
  }

  /** Replaces the rule (a saved one was chosen). */
  load(rule: RenameRule) {
    this.rule = copyRule(rule);
  }

  #ask(rule: RenameRule) {
    this.#sequence += 1;
    const mine = this.#sequence;
    this.pending = true;
    clearTimeout(this.#timer);

    this.#timer = setTimeout(async () => {
      let preview: RenamePreview | undefined;
      let failure: string | undefined;
      try {
        preview = await this.#ipc.invoke('rename_preview', { path: this.#path, names: [...this.#names], rule });
      } catch (error) {
        failure = IpcFailure.from(error).message;
      }

      if (mine === this.#sequence) {
        this.preview = preview ?? this.preview;
        this.failure = failure;
        this.pending = false;
      }
    }, PREVIEW_DELAY_MS);
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
