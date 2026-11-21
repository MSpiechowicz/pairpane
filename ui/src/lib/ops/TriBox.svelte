<script lang="ts">
  import type { Tri } from './jobs';

  let {
    value = $bindable(),
    label,
    mixed,
    onkeydown,
  }: {
    /** Left alone (shown as a dash), set, or cleared. */
    value: Tri;
    /** The accessible name; the box has no text beside it when it sits in a grid. */
    label: string;
    /** The items differ in this one: a click goes through "leave alone" too. Otherwise it only sets and clears. */
    mixed: boolean;
    onkeydown?: (event: KeyboardEvent) => void;
  } = $props();

  /** The box shows the state: checked, empty, or (left alone) indeterminate. */
  function show(input: HTMLInputElement) {
    input.checked = value === 'on';
    input.indeterminate = value === 'keep';
  }

  const ROUND = { keep: 'on', on: 'off', off: 'keep' } as const;
  const FLIP = { keep: 'on', on: 'off', off: 'on' } as const;
</script>

<input type="checkbox" aria-label={label} {onkeydown} onclick={() => (value = (mixed ? ROUND : FLIP)[value])} {@attach show} />

<style>
  input {
    margin: 0;
    accent-color: var(--accent);
  }
</style>
