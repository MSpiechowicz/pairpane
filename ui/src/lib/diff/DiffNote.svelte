<script lang="ts">
  import { displayName } from '$lib/panes/format';

  let { text, role = 'status' }: { text: string; role?: 'status' | 'alert' | 'note' } = $props();
</script>

<p class="note" class:error={role === 'alert'} {role}>{displayName(text)}</p>

<style>
  .note {
    margin: 0;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--panel-2);
    color: var(--dim);
    font-size: 12px;
  }

  .error {
    color: var(--danger);
  }
</style>
