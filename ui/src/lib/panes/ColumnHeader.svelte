<script lang="ts">
  import { preferences } from '$lib/shell/preferences.svelte';

  import { shows } from './columns';
  import type { SortColumn } from './order';
  import type { PaneView } from './view.svelte';

  let { view }: { view: PaneView } = $props();

  /** `cell` names the column in the narrow layout, which drops Ext and Modified. */
  const columns: { column: SortColumn; label: string; cell: string }[] = [
    { column: 'name', label: 'Name', cell: 'nm' },
    { column: 'ext', label: 'Ext', cell: 'ex' },
    { column: 'size', label: 'Size', cell: 'sz' },
    { column: 'modified', label: 'Modified', cell: 'dt' },
  ];

  const shown = $derived(columns.filter(({ column }) => shows(column, preferences.value)));
</script>

<div class="head" role="group" aria-label="Sort by">
  <span></span>
  {#each shown as { column, label, cell } (column)}
    <div class={cell}>
      <button type="button" onclick={() => view.sortBy(column)}>
        {label}
        {#if column === view.column}
          <span class="arrow" aria-hidden="true">{view.desc ? '▼' : '▲'}</span>
          <span class="sr-only">sorted {view.desc ? 'descending' : 'ascending'}</span>
        {/if}
      </button>
    </div>
  {/each}
</div>

<style>
  .head {
    display: grid;
    grid-template-columns: var(--pane-cols);
    align-items: center;
    gap: 8px;
    height: 25px;
    padding: 0 10px 0 12px;
    border-bottom: 1px solid var(--line);
    font-size: 11px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--dim);
    user-select: none;
  }

  .head button {
    display: inline-flex;
    gap: 3px;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
  }

  .head button:hover {
    color: var(--text);
  }

  .arrow {
    font-size: 8px;
    line-height: 1.6;
  }

  .sz {
    text-align: right;
  }

  .sz button {
    margin-left: auto;
  }

  @media (max-width: 760px) {
    .ex,
    .dt {
      display: none;
    }
  }
</style>
