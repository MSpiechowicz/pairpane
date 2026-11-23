import type { KeyOverride } from '$lib/ipc/bindings/KeyOverride';

import { ACTIONS, type Action, BINDINGS, type Binding, type Chord, keysOf, refusalOf, shortcutFor, written } from './shortcuts';

const same = (a: Chord, b: Chord) => a.key === b.key && !!a.ctrl === !!b.ctrl && !!a.alt === !!b.alt && !!a.shift === !!b.shift;

const isAction = (name: string): name is Action => (ACTIONS as readonly string[]).includes(name);

/** What setting a key did: the action that had it before, when one did. */
export interface Taken {
  from: Action | undefined;
}

/**
 * The keymap in use: the default keys of every action (the Total Commander ones, see `BINDINGS`) with those the user set
 * in their place. An action the user set has only the keys set (none: no key). A key belongs to one action at most: setting
 * it takes it from the action that had it. What the user set is kept with the session (`overrides`).
 */
export class Keymap {
  /** The keys the user set, by action. */
  #custom = $state.raw(new Map<Action, readonly Chord[]>());

  /** Every binding in use, the user's first. */
  readonly #bindings = $derived.by(() => {
    const custom = this.#custom;
    const own: Binding[] = [...custom].flatMap(([action, chords]) => chords.map((chord) => ({ ...chord, action })));
    const defaults = BINDINGS.filter((binding) => !custom.has(binding.action) && !own.some((taken) => same(taken, binding)));

    return [...own, ...defaults];
  });

  get bindings(): readonly Binding[] {
    return this.#bindings;
  }

  /** What the user set, to be saved. */
  get overrides(): KeyOverride[] {
    return [...this.#custom].map(([action, chords]) => ({
      action,
      keys: chords.map(({ key, ctrl, alt, shift }) => ({ key, ctrl: !!ctrl, alt: !!alt, shift: !!shift })),
    }));
  }

  /** The action a key press stands for (see `shortcutFor`). */
  shortcutFor(event: KeyboardEvent): Action | undefined {
    return shortcutFor(event, this.#bindings);
  }

  /** The keys of `action` as they are written, when it has any. */
  keysOf(action: Action): string | undefined {
    return keysOf(action, this.#bindings);
  }

  /** Every key of `action`, as they are written. */
  allKeysOf(action: Action): string[] {
    return this.#bindings.filter((binding) => binding.action === action).map(written);
  }

  /** Whether the user set the keys of `action`. */
  isCustom(action: Action): boolean {
    return this.#custom.has(action);
  }

  /** Whether `action` has other keys than its default ones (it was set, or a key of it was taken). */
  differs(action: Action): boolean {
    const defaults = BINDINGS.filter((binding) => binding.action === action).map(written);

    return this.isCustom(action) || this.allKeysOf(action).join() !== defaults.join();
  }

  /**
   * Brings back what the user set: what is not an action, a key an action cannot have or one an earlier action has is left
   * out, and an action none of whose keys is left (while it had some) keeps its defaults.
   */
  restore(saved: readonly KeyOverride[]) {
    const custom = new Map<Action, readonly Chord[]>();
    const taken: Chord[] = [];

    for (const { action, keys } of saved) {
      if (!isAction(action) || custom.has(action)) {
        continue;
      }

      const usable = keys
        .map((chord) => ({ ...chord, key: chord.key.length === 1 ? chord.key.toLowerCase() : chord.key }))
        .filter((chord) => !(chord.ctrl && chord.alt) && refusalOf(chord) === undefined && !taken.some((other) => same(other, chord)));
      if (usable.length > 0 || keys.length === 0) {
        custom.set(action, usable);
        taken.push(...usable);
      }
    }

    this.#custom = custom;
  }

  /** Makes `chord` the key of `action`, taken from the action that had it; refused (with why) when it cannot be one. */
  set(action: Action, chord: Chord): Taken | string {
    const refused = refusalOf(chord);
    if (refused !== undefined) {
      return refused;
    }

    const from = this.#bindings.find((binding) => same(binding, chord))?.action;
    // The palette opens this dialog: its last key is never taken (the top bar opens the dialog too, but a key is kept).
    if (from === 'palette' && action !== 'palette' && this.allKeysOf('palette').length === 1) {
      return `${written(chord)} is the only key of the command palette: give it another key first.`;
    }

    const custom = new Map(this.#custom);
    if (from !== undefined && from !== action) {
      const left = this.#bindings.filter((binding) => binding.action === from && !same(binding, chord));
      custom.set(from, left);
    }
    custom.set(action, [chord]);
    this.#custom = custom;

    return { from: from === action ? undefined : from };
  }

  /** Gives `action` its default keys again (a key of them that another action took stays with that one). */
  reset(action: Action) {
    const custom = new Map(this.#custom);
    custom.delete(action);
    this.#custom = custom;
  }

  /** Gives every action its default keys again. */
  resetAll() {
    this.#custom = new Map();
  }
}

/** The keymap of the app. */
export const keymap = new Keymap();
