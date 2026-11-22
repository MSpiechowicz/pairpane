<script lang="ts">
  import EditBanners from '$lib/editor/EditBanners.svelte';
  import { ENDING_LABELS, type EditSession } from '$lib/editor/session.svelte';
  import { displayName } from '$lib/panes/format';

  let {
    session,
    label,
    onSave,
  }: {
    session: EditSession;
    /** Which side it is, for assistive technology. */
    label: string;
    onSave: () => void;
  } = $props();
</script>

<div class="side" role="group" aria-label={label}>
  <div class="head">
    <span class="name">{displayName(session.name)}</span>
    {#if session.dirty}
      <span class="dirty" role="img" aria-label="Unsaved changes">●</span>
    {/if}
    <span class="facts">{displayName(session.encoding)} · {ENDING_LABELS[session.lineEnding]}</span>
    <span class="facts cursor" aria-label="Cursor position">Ln {session.cursor.line}, Col {session.cursor.col}</span>
    <button class="btn btn-primary save" type="button" disabled={session.readOnly || session.saving} onclick={onSave}>Save</button>
  </div>
  <EditBanners {session} />
</div>

<style>
  .side {
    flex: 1 1 0;
    min-width: 0;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
  }

  .name {
    min-width: 0;
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dirty {
    margin-left: -2px;
    color: var(--sel);
  }

  .facts {
    color: var(--dim);
    font-size: 12px;
    white-space: nowrap;
  }

  .cursor {
    margin-left: auto;
    font-family: var(--font-mono);
    font-size: 11px;
  }

  .save {
    padding: 2px 10px;
  }
</style>
