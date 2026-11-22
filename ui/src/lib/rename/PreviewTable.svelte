<script lang="ts">
  import type { RenameRow } from '$lib/ipc/bindings/RenameRow';
  import { displayName } from '$lib/panes/format';
  import VirtualRows from '$lib/panes/VirtualRows.svelte';

  import { PROBLEMS } from './problems';

  let { rows, stale = false }: { rows: readonly RenameRow[]; stale?: boolean } = $props();
</script>

<div class="table" class:stale role="table" aria-label="Preview" aria-busy={stale}>
  <div class="head" role="row">
    <span role="columnheader">Now</span>
    <span aria-hidden="true"></span>
    <span role="columnheader">Renamed to</span>
    <span role="columnheader">Problem</span>
  </div>
  <div class="body">
    <VirtualRows count={rows.length}>
      {#snippet row(index)}
        {@const item = rows[index]}
        {#if item !== undefined}
          <div class="row" class:unchanged={item.from === item.to && item.problem === null} class:bad={item.problem !== null} role="row">
            <span class="name" role="cell">{displayName(item.from)}</span>
            <span class="arrow" aria-hidden="true">→</span>
            <span class="name" role="cell">{displayName(item.to)}</span>
            <span class="why" role="cell">{item.problem === null ? '' : PROBLEMS[item.problem]}</span>
          </div>
        {/if}
      {/snippet}
    </VirtualRows>
  </div>
</div>

<style>
  .table {
    border: 1px solid var(--line);
    border-radius: 7px;
    background: var(--bg);
    overflow: hidden;
  }

  .stale .body {
    opacity: 0.6;
  }

  .head,
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 20px minmax(0, 1fr) minmax(0, 0.8fr);
    align-items: center;
    gap: 8px;
    height: 24px;
    padding: 0 10px;
  }

  .head {
    border-bottom: 1px solid var(--line);
    background: var(--panel-2);
    color: var(--dim);
    font-size: 11.5px;
  }

  .body {
    height: 200px;
  }

  .row {
    border-left: 2px solid transparent;
    font-size: 12.5px;
  }

  .row.unchanged {
    color: var(--dim);
  }

  .row.bad {
    border-left-color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    unicode-bidi: isolate; /* a name may be written right to left: it is set apart from the text around it */
  }

  .arrow {
    color: var(--dim);
    text-align: center;
  }

  .why {
    overflow: hidden;
    color: var(--danger);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
