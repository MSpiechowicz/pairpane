<script lang="ts">
  import type { RenameRow } from '$lib/ipc/bindings/RenameRow';
  import { displayName } from '$lib/panes/format';
  import VirtualRows from '$lib/panes/VirtualRows.svelte';

  import { PROBLEMS } from './problems';
  import '../ops/table.css';

  let { rows, stale = false }: { rows: readonly RenameRow[]; stale?: boolean } = $props();
</script>

<div class="table dialog-table" class:stale role="table" aria-label="Preview" aria-busy={stale}>
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
  .stale .body {
    opacity: 0.6;
  }

  .head,
  .row {
    grid-template-columns: minmax(0, 1fr) 20px minmax(0, 1fr) minmax(0, 0.8fr);
  }

  .body {
    height: 200px;
  }

  .row.unchanged {
    color: var(--dim);
  }

  .row.bad {
    border-left-color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
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
