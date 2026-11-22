<script lang="ts">
  import type { CompareFacts } from '$lib/ipc/bindings/CompareFacts';
  import { displayName, formatDate, formatSize } from '$lib/panes/format';
  import VirtualRows from '$lib/panes/VirtualRows.svelte';

  import { type SyncAction, STATUS_TEXT, whyNot } from './plan';
  import type { Syncer } from './syncer.svelte';
  import '../ops/table.css';

  let { syncer }: { syncer: Syncer } = $props();

  /** The choices of a row, in the order the arrows point: to the left, nowhere, to the right. */
  const CHOICES: readonly { action: SyncAction; glyph: string; text: string }[] = [
    { action: 'toLeft', glyph: '←', text: 'Copy to the left' },
    { action: 'skip', glyph: '–', text: 'Skip' },
    { action: 'toRight', glyph: '→', text: 'Copy to the right' },
  ];

  const allPicked = $derived(syncer.count > 0 && syncer.picked === syncer.count);

  /** What one side holds, as a line: its kind or size, and its time. */
  function facts(side: CompareFacts | null): string {
    if (side === null) {
      return '';
    }

    if (side.link) {
      return 'link';
    }

    if (side.kind === 'dir') {
      return side.files > 0 ? `folder · ${side.files.toLocaleString('en-US')} files · ${formatSize(side.bytes)}` : 'folder';
    }

    return `${formatSize(side.size)} · ${formatDate(side.modified)}`;
  }
</script>

<div class="dialog-table" role="table" aria-label="Differences" aria-busy={syncer.running}>
  <div class="head" role="row">
    <span role="columnheader">
      <input type="checkbox" aria-label="Pick every row shown" checked={allPicked} onchange={(event) => syncer.pickAll(event.currentTarget.checked)} />
    </span>
    <span role="columnheader">Path</span>
    <span role="columnheader">Left</span>
    <span role="columnheader">Action</span>
    <span role="columnheader">Right</span>
    <span role="columnheader">State</span>
  </div>
  <div class="body">
    <VirtualRows count={syncer.count}>
      {#snippet row(index)}
        {@const item = syncer.at(index)}
        {#if item !== undefined}
          {@const action = syncer.actionOf(item)}
          {@const picked = syncer.pickedOf(item)}
          <div class="row {item.row.status}" class:picked role="row">
            <span role="cell">
              <input type="checkbox" aria-label="Pick {displayName(item.row.path)}" checked={picked} onchange={() => syncer.pick(item)} />
            </span>
            <span class="name" role="cell" title={displayName(item.row.path)}>{displayName(item.row.path)}</span>
            <span class="facts" role="cell">{facts(item.row.left)}</span>
            <span class="choices" role="cell">
              {#each CHOICES as choice (choice.action)}
                {@const why = choice.action === 'skip' ? undefined : whyNot(item.row, choice.action, syncer.archives)}
                <button
                  type="button"
                  class:on={action === choice.action}
                  aria-pressed={action === choice.action}
                  aria-label="{choice.text}: {displayName(item.row.path)}"
                  title={why ?? choice.text}
                  disabled={why !== undefined}
                  onclick={() => syncer.choose(item, choice.action)}>{choice.glyph}</button
                >
              {/each}
            </span>
            <span class="facts" role="cell">{facts(item.row.right)}</span>
            <span class="state" role="cell">{STATUS_TEXT[item.row.status]}</span>
          </div>
        {/if}
      {/snippet}
    </VirtualRows>
  </div>
</div>

<style>
  .head,
  .row {
    grid-template-columns: 20px minmax(0, 1fr) minmax(0, 150px) 78px minmax(0, 150px) 82px;
  }

  .body {
    height: 230px;
  }

  .row.picked {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }

  .row.differs {
    border-left-color: var(--sel);
  }

  .row.leftOnly,
  .row.rightOnly {
    border-left-color: var(--accent);
  }

  .row.leftNewer,
  .row.rightNewer {
    border-left-color: var(--ok);
  }

  .row.equal {
    color: var(--dim);
  }

  input[type='checkbox'] {
    margin: 0;
  }

  .facts,
  .state {
    color: var(--dim);
    font-size: 11.5px;
  }

  .facts {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .state {
    white-space: nowrap;
  }

  .choices {
    display: flex;
    gap: 2px;
  }

  .choices button {
    width: 24px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: 5px;
    background: none;
    color: var(--dim);
    cursor: default;
  }

  .choices button:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--text);
  }

  .choices button.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--text);
  }

  .choices button:disabled {
    opacity: 0.35;
  }
</style>
