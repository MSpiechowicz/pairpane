<script lang="ts">
  import { stemLength } from '$lib/ops/targets';

  import type { InlineRename } from './inlineRename.svelte';

  let { rename }: { rename: InlineRename } = $props();

  /** The field has the focus; a name that was not edited yet has its stem selected, so typing keeps the extension. */
  function show(input: HTMLInputElement) {
    const leave = rename.mount(input);
    const edit = rename.current;
    if (edit !== undefined && rename.value === edit.name) {
      input.setSelectionRange(0, stemLength(edit.name, edit.dir));
    } else {
      input.setSelectionRange(rename.value.length, rename.value.length);
    }

    return leave;
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing) {
      return;
    }

    if (event.key === 'Enter') {
      event.preventDefault();
      void rename.commit();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      rename.cancel();
    }
  }
</script>

<input
  class="pane-field"
  type="text"
  spellcheck="false"
  autocomplete="off"
  aria-label="New name"
  readonly={rename.busy}
  bind:value={rename.value}
  {onkeydown}
  onblur={() => rename.cancel()}
  {@attach show}
/>

<style>
  /* Spans the name and extension columns (just the name when the extension is not shown: see Pane). */
  input {
    grid-column: 2 / var(--rename-end, 4);
    min-width: 0;
    user-select: text;
  }

  @media (max-width: 760px) {
    input {
      grid-column: 2;
    }
  }
</style>
