<script lang="ts">
  import type { MdBlock as Block } from '$lib/ipc/bindings/MdBlock';

  import MdBlock from './MdBlock.svelte';
  import MdInline from './MdInline.svelte';

  let { blocks }: { blocks: Block[] } = $props();
</script>

<!-- Elements and text only (see MdInline). -->
{#each blocks as block, at (at)}
  {#if block.kind === 'paragraph'}
    <p><MdInline inlines={block.inlines} /></p>
  {:else if block.kind === 'heading'}
    <svelte:element this={`h${Math.min(6, Math.max(1, block.level))}`}><MdInline inlines={block.inlines} /></svelte:element>
  {:else if block.kind === 'codeBlock'}
    <pre><code data-language={block.language ?? undefined}>{block.text}</code></pre>
  {:else if block.kind === 'quote'}
    <blockquote><MdBlock blocks={block.blocks} /></blockquote>
  {:else if block.kind === 'list'}
    <svelte:element this={block.ordered ? 'ol' : 'ul'} start={block.ordered ? (block.start ?? undefined) : undefined}>
      {#each block.items as item, index (index)}
        <li class:task={item.checked !== null}>
          {#if item.checked !== null}
            <input type="checkbox" checked={item.checked} disabled aria-label={item.checked ? 'Done' : 'Not done'} />
          {/if}
          <div><MdBlock blocks={item.blocks} /></div>
        </li>
      {/each}
    </svelte:element>
  {:else if block.kind === 'table'}
    <table>
      <thead>
        <tr>
          {#each block.header as cell, column (column)}
            <th><MdInline inlines={cell} /></th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#each block.rows as row, index (index)}
          <tr>
            {#each row as cell, column (column)}
              <td><MdInline inlines={cell} /></td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>
  {:else}
    <hr />
  {/if}
{/each}

<style>
  p,
  blockquote,
  pre,
  table,
  ul,
  ol {
    margin: 0 0 12px;
  }

  pre {
    padding: 10px 12px;
    overflow-x: auto;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel-2);
    font: 12.5px/18px var(--font-mono);
  }

  blockquote {
    padding-left: 12px;
    border-left: 3px solid var(--line);
    color: var(--dim);
  }

  ul,
  ol {
    padding-left: 24px;
  }

  .task {
    list-style: none;
    display: flex;
    gap: 6px;
    margin-left: -20px;
  }

  table {
    border-collapse: collapse;
  }

  th,
  td {
    padding: 4px 10px;
    border: 1px solid var(--line);
    text-align: left;
  }

  th {
    background: var(--panel-2);
  }

  hr {
    border: 0;
    border-top: 1px solid var(--line);
  }
</style>
