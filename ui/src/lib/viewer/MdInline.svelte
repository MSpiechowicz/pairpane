<script lang="ts">
  import type { MdInline as Inline } from '$lib/ipc/bindings/MdInline';

  import MdInline from './MdInline.svelte';

  let { inlines }: { inlines: Inline[] } = $props();
</script>

<!-- Elements and text only: what the file says never becomes markup, and a link is never one to follow. -->
{#each inlines as inline, at (at)}
  {#if inline.kind === 'text'}
    {inline.text}
  {:else if inline.kind === 'code'}
    <code>{inline.text}</code>
  {:else if inline.kind === 'emphasis'}
    <em><MdInline inlines={inline.children} /></em>
  {:else if inline.kind === 'strong'}
    <strong><MdInline inlines={inline.children} /></strong>
  {:else if inline.kind === 'strike'}
    <del><MdInline inlines={inline.children} /></del>
  {:else if inline.kind === 'link'}
    <span class="link" title={inline.href ?? undefined}><MdInline inlines={inline.children} /></span>
  {:else if inline.kind === 'image'}
    <span class="alt">{inline.alt}</span>
  {:else}
    <br />
  {/if}
{/each}

<style>
  code {
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--panel-2);
    font: 0.92em var(--font-mono);
  }

  .link {
    color: var(--accent);
    text-decoration: underline;
    cursor: default;
  }

  .alt {
    color: var(--dim);
    font-style: italic;
  }
</style>
