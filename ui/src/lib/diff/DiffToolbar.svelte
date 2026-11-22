<script lang="ts">
  import type { Snippet } from 'svelte';

  import { displayName } from '$lib/panes/format';
  import '$lib/shell/overlayBar.css';

  let {
    summary,
    disabled,
    onPrevious,
    onNext,
    onClose,
    children,
  }: {
    /** What the comparison found, in a few words. */
    summary: string;
    /** There is nothing to move to. */
    disabled: boolean;
    onPrevious: () => void;
    onNext: () => void;
    onClose: () => void;
    /** More controls, between the moves and Close. */
    children?: Snippet;
  } = $props();
</script>

<div class="overlay-bar" role="toolbar" aria-label="Compare files">
  <span class="overlay-bar-name" role="status">{displayName(summary)}</span>

  <button class="btn" type="button" aria-label="Previous difference" {disabled} onclick={onPrevious}>Previous <kbd>Shift+F7</kbd></button>
  <button class="btn" type="button" aria-label="Next difference" {disabled} onclick={onNext}>Next <kbd>F7</kbd></button>
  {@render children?.()}

  <button class="btn close" type="button" aria-label="Close" onclick={onClose}>Close <kbd>Esc</kbd></button>
</div>

<style>
  .close {
    margin-left: auto;
  }
</style>
