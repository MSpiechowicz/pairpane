<script lang="ts">
  import type { LineEnding } from '$lib/ipc/bindings/LineEnding';
  import { displayName } from '$lib/panes/format';
  import '$lib/shell/overlayBar.css';
  import { encodingChoices } from '$lib/viewer/encodings';

  import type { Editor } from './editor.svelte';
  import { type EditSession, isUnicode } from './session.svelte';

  let { editor, session }: { editor: Editor; session: EditSession } = $props();

  const ENDINGS: readonly { value: LineEnding; label: string }[] = [
    { value: 'lf', label: 'LF' },
    { value: 'crlf', label: 'CRLF' },
    { value: 'cr', label: 'CR' },
  ];

  /** The select shows what the session holds: a choice that was not taken (the file would not read so) goes back. */
  async function chooseEncoding(select: HTMLSelectElement) {
    await editor.changeEncoding(select.value);
    select.value = session.encoding;
  }
</script>

<div class="overlay-bar" role="toolbar" aria-label="Editor">
  <span class="overlay-bar-name" aria-hidden="true">{displayName(session.name)}</span>
  {#if session.dirty}
    <span class="dirty" role="img" aria-label="Unsaved changes">●</span>
  {/if}

  {#if session.status === 'ready'}
    <select aria-label="Encoding" value={session.encoding} onchange={(event) => void chooseEncoding(event.currentTarget)}>
      {#each encodingChoices(session.encoding) as encoding (encoding)}
        <option value={encoding}>{encoding}</option>
      {/each}
    </select>

    <select aria-label="Line ending" value={session.lineEnding} onchange={(event) => session.setLineEnding(event.currentTarget.value as LineEnding)}>
      {#each ENDINGS as { value, label } (value)}
        <option {value}>{label}</option>
      {/each}
    </select>

    {#if isUnicode(session.encoding)}
      <label class="check">
        <input type="checkbox" checked={session.bom} onchange={(event) => session.setBom(event.currentTarget.checked)} />
        Byte order mark
      </label>
    {/if}

    <button class="btn btn-primary save" type="button" disabled={session.readOnly || session.saving} onclick={() => void editor.save()}>
      Save
    </button>
  {/if}

  <button class="btn" type="button" aria-label="Close" onclick={() => void editor.requestClose()}>Close <kbd>Esc</kbd></button>
</div>

<style>
  .dirty {
    margin-left: -4px;
    color: var(--sel);
  }

  select {
    height: 26px;
    padding: 0 6px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel-2);
    color: inherit;
    font: inherit;
  }

  .save {
    margin-left: auto;
  }
</style>
