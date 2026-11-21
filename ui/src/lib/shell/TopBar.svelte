<script lang="ts">
  import type { Snippet } from 'svelte';

  import type { FileAction } from '$lib/ops/dialogs';

  import FilesMenu from './FilesMenu.svelte';
  import { shortcutFor } from './shortcuts';
  import type { Theme } from './theme.svelte';

  let {
    version,
    failure,
    theme,
    nativeMenu,
    showHidden,
    onCheckForUpdates,
    onToggleHidden,
    onFiles,
  }: {
    version: string | undefined;
    failure: string | undefined;
    theme: Theme;
    /** macOS: the native app menu holds "Check for Updates…", so the top bar does not repeat it. */
    nativeMenu: boolean;
    /** Whether the panes list hidden files. */
    showHidden: boolean;
    onCheckForUpdates: () => void;
    onToggleHidden: () => void;
    /** An item of the Files menu was chosen; it does what its key does. */
    onFiles: (action: FileAction) => void;
  } = $props();

  /** The menus that hold nothing yet. */
  const MENUS = ['Mark', 'Commands', 'Net', 'View'] as const;
  const THEME_LABEL = { system: 'System', light: 'Light', dark: 'Dark' } as const;

  type Popup = 'files' | 'settings';

  /** The popup that is open; opening one closes the other. */
  let open = $state<Popup | undefined>();

  const toggle = (popup: Popup) => (open = open === popup ? undefined : popup);

  function checkForUpdates() {
    open = undefined;
    onCheckForUpdates();
  }

  function chooseFile(action: FileAction) {
    open = undefined;
    onFiles(action);
  }

  function closeOnOutside(event: MouseEvent) {
    if (open && !(event.target as Element).closest('.has-popup')) {
      open = undefined;
    }
  }

  /** Escape closes the popup, and so does a shortcut: its action (a dialog, a pane change) takes over. */
  function closeOnKey(event: KeyboardEvent) {
    if (open && (event.key === 'Escape' || shortcutFor(event) !== undefined)) {
      open = undefined;
    }
  }
</script>

<svelte:window onclick={closeOnOutside} onkeydown={closeOnKey} />

{#snippet popup(id: Popup, label: string, menu: Snippet)}
  <div class="has-popup">
    <button type="button" aria-haspopup="menu" aria-expanded={open === id} onclick={() => toggle(id)}>{label}</button>
    {#if open === id}
      {@render menu()}
    {/if}
  </div>
{/snippet}

{#snippet files()}
  <FilesMenu onChoose={chooseFile} />
{/snippet}

{#snippet settings()}
  <div class="popup" role="menu">
    <button class="popup-item" type="button" role="menuitem" onclick={checkForUpdates}>Check for Updates…</button>
  </div>
{/snippet}

<header class="top">
  <div class="brand">
    <img src="/favicon.svg" alt="" width="22" height="13" />
    <h1>Pairpane</h1>
    {#if version}
      <span class="version">v{version}</span>
    {:else if failure}
      <span class="failure" role="alert">Could not load app info: {failure}</span>
    {/if}
  </div>

  <nav class="menu" aria-label="Main menu">
    {@render popup('files', 'Files', files)}
    {#each MENUS as name (name)}
      <button class="placeholder" type="button">{name}</button>
    {/each}
    <!-- macOS: "Check for Updates…" is in the native app menu, the only thing Settings holds so far. -->
    {#if !nativeMenu}
      {@render popup('settings', 'Settings', settings)}
    {/if}
  </nav>

  <button class="search" type="button">
    <span>Search files or run a command…</span>
    <kbd>{nativeMenu ? '⌘K' : 'Ctrl K'}</kbd>
  </button>

  <div class="tools">
    <button type="button">⇆ Compare</button>
    <button type="button">Σ Sizes</button>
    <button type="button" class:on={showHidden} aria-pressed={showHidden} onclick={onToggleHidden}>Hidden</button>
    <button type="button" title="Theme: {THEME_LABEL[theme.mode]}" onclick={() => theme.cycle()}>
      <svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="5.5" /><path d="M8 2.5v11a5.5 5.5 0 0 0 0-11z" /></svg>
      <span class="sr-only">Theme: {THEME_LABEL[theme.mode]}</span>
    </button>
  </div>
</header>

<style>
  .top {
    display: flex;
    align-items: center;
    gap: 14px;
    height: 44px;
    padding: 0 10px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
    white-space: nowrap;
  }

  .brand {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  h1 {
    margin: 0;
    font-size: inherit;
    font-weight: 700;
  }

  .version {
    font: 11px var(--font-mono);
    color: var(--dim);
  }

  .failure {
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--danger);
  }

  .menu,
  .tools {
    flex: none;
    display: flex;
    gap: 2px;
  }

  .menu > button,
  .menu > .has-popup > button,
  .tools button {
    height: 30px;
    padding: 0 9px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: none;
    color: var(--dim);
    cursor: default;
  }

  .menu > button:hover,
  .menu > .has-popup > button:hover,
  .tools button:hover {
    border-color: var(--line);
    color: var(--text);
  }

  .tools button.on {
    color: var(--accent-2);
  }

  .tools svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
  }

  .tools svg path {
    fill: currentColor;
  }

  .search {
    flex: 1 1 160px;
    min-width: 0;
    max-width: 460px;
    height: 30px;
    margin: 0 auto;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 0 8px 0 12px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--bg);
    color: var(--dim);
  }

  .search:hover {
    border-color: var(--accent);
  }

  .search span {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* The menus that hold nothing yet give way first; Files and Settings (the update check) stay at every width. */
  @media (max-width: 1100px) {
    .placeholder {
      display: none;
    }
  }

  @media (max-width: 760px) {
    .tools {
      display: none;
    }
  }
</style>
