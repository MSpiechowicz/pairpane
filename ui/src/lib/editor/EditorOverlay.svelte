<script lang="ts">
  import { untrack } from 'svelte';

  import { displayName } from '$lib/panes/format';
  import ChoiceDialog from '$lib/shell/ChoiceDialog.svelte';
  import Modal from '$lib/shell/Modal.svelte';
  import { encodingChoices } from '$lib/viewer/encodings';

  import type { Editor } from './editor.svelte';
  import EditorToolbar from './EditorToolbar.svelte';
  import type { EditSession } from './session.svelte';
  import { type EditorSurface, loadSurface } from './surface';

  let { editor, session }: { editor: Editor; session: EditSession } = $props();

  /** What a file that would not read is read as next. */
  let chosen = $state(untrack(() => (session.unreadable === 'windows-1252' ? 'ISO-8859-2' : 'windows-1252')));

  const endings = { lf: 'LF', crlf: 'CRLF', cr: 'CR' } as const;

  /** Puts the editing widget (loaded on first use) in `node`; the session hears of its changes. */
  function mount(node: HTMLElement) {
    let surface: EditorSurface | undefined;
    let gone = false;

    loadSurface()
      .then(({ createSurface }) => {
        if (gone) {
          return;
        }

        surface = createSurface({
          parent: node,
          name: session.name,
          text: session.text(),
          readOnly: session.readOnly,
          onSave: () => void editor.save(),
        });
        session.attach(surface);
        surface.focus();
      })
      .catch(() => (session.failure = 'The editor could not be loaded.'));

    return () => {
      gone = true;
      if (surface !== undefined) {
        session.detach();
        surface.destroy();
      }
    };
  }

  /** Ctrl+S saves from anywhere in the overlay (in the text it is the surface's own); the question on top has the keyboard. */
  function onkeydown(event: KeyboardEvent) {
    if (event.key.toLowerCase() === 's' && event.ctrlKey && !event.altKey && !event.metaKey && !event.defaultPrevented) {
      event.preventDefault();
      if (editor.question === undefined && session.status === 'ready') {
        void editor.save();
      }
    }
  }
</script>

<svelte:window {onkeydown} />

<Modal title={displayName(session.name)} variant="full" onClose={() => void editor.requestClose()}>
  <div class="editor">
    <EditorToolbar {editor} {session} />

    {#if session.status === 'undecodable'}
      <div class="picker">
        <p role="alert">{displayName(session.name)} is not valid {session.unreadable} text.</p>
        <label>
          Reopen as
          <select data-autofocus value={chosen} onchange={(event) => (chosen = event.currentTarget.value)}>
            {#each encodingChoices(session.unreadable) as encoding (encoding)}
              <option value={encoding}>{encoding}</option>
            {/each}
          </select>
        </label>
        <button class="btn btn-primary" type="button" onclick={() => void editor.changeEncoding(chosen)}>Reopen</button>
        {#if session.failure}
          <p role="alert">{displayName(session.failure)}</p>
        {/if}
      </div>
    {:else}
      {#if session.readOnly}
        <p class="banner" role="status">This file is read-only: it cannot be changed or saved here.</p>
      {/if}
      {#if session.mixedEndings}
        <p class="banner" role="status">
          This file mixes line endings; saving writes all of them as {endings[session.lineEnding]}.
        </p>
      {/if}
      {#if session.note}
        <p class="banner" role="status">{displayName(session.note)}</p>
      {/if}
      {#if session.failure}
        <p class="banner error" role="alert">{displayName(session.failure)}</p>
      {/if}

      <div class="host" {@attach mount}></div>
      <div class="status" aria-label="Cursor position">Ln {session.cursor.line}, Col {session.cursor.col}</div>
    {/if}
  </div>
</Modal>

{#if editor.question}
  {@const question = editor.question}
  <ChoiceDialog
    title={question.title}
    message={question.message}
    confirm={question.confirm}
    alternate={question.alternate}
    danger={question.danger}
    onConfirm={() => question.answer('confirm')}
    onAlternate={() => question.answer('alternate')}
    onCancel={() => question.answer('cancel')}
  />
{/if}

<style>
  .editor {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  .host {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .banner {
    margin: 0;
    padding: 6px 12px;
    border-bottom: 1px solid var(--line);
    background: var(--panel-2);
    color: var(--dim);
    font-size: 12px;
  }

  .banner.error {
    color: var(--danger);
  }

  .status {
    padding: 3px 12px;
    border-top: 1px solid var(--line);
    background: var(--panel);
    color: var(--dim);
    font: 11px var(--font-mono);
  }

  .picker {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding: 24px 16px;
  }

  .picker p {
    flex-basis: 100%;
    margin: 0;
  }

  .picker p[role='alert'] {
    color: var(--danger);
  }

  .picker select {
    height: 26px;
    margin-left: 6px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel-2);
    color: inherit;
    font: inherit;
  }
</style>
