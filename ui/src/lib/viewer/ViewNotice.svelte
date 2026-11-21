<script lang="ts">
  let {
    error,
    text,
  }: {
    /** Why the view cannot go on: shown as an alert. */
    error?: string;
    /** What there is to say otherwise ("The file is empty."). */
    text?: string;
  } = $props();
</script>

{#if error}
  <p class="notice" role="alert">{error}</p>
{:else if text}
  <p class="notice">{text}</p>
{/if}

<style>
  .notice {
    position: absolute;
    right: 24px;
    bottom: 8px;
    left: 12px;
    margin: 0;
    padding: 6px 10px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel-2);
    color: var(--dim);
    font-size: 12px;
  }

  .notice[role='alert'] {
    border-color: color-mix(in srgb, var(--danger) 40%, var(--line));
    color: var(--danger);
  }
</style>
