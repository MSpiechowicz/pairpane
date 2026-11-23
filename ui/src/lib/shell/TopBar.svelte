<script lang="ts">
  import type { Snippet } from 'svelte';

  import { displayName } from '$lib/panes/format';
  import type { Layout } from '$lib/panes/layout';

  import FilesMenu from './FilesMenu.svelte';
  import type { FilesChoice } from './menu';
  import { keymap } from './keymap.svelte';
  import type { Theme } from './theme.svelte';
  import ViewMenu from './ViewMenu.svelte';

  let {
    version,
    failure,
    theme,
    nativeMenu,
    showHidden,
    layout,
    onLayout,
    onCheckForUpdates,
    onSettings,
    onEditSettings,
    onKeyboardShortcuts,
    onToggleHidden,
    onFiles,
    onFind,
    onCompare,
    onSync,
  }: {
    version: string | undefined;
    failure: string | undefined;
    theme: Theme;
    /** macOS: the native app menu holds "Check for Updates…", so the Settings menu does not repeat it. */
    nativeMenu: boolean;
    /** Whether the panes list hidden files. */
    showHidden: boolean;
    /** How the active pane shows its folder. */
    layout: Layout;
    /** An item of the View menu was chosen; it does what its key does. */
    onLayout: (layout: Layout) => void;
    onCheckForUpdates: () => void;
    /** "Settings…": the settings dialog (the palette opens it too). */
    onSettings: () => void;
    /** "Edit Settings…": the settings file opens in the system's editor for it. */
    onEditSettings: () => void;
    /** "Keyboard Shortcuts…": the dialog that changes the keys (the palette opens it too). */
    onKeyboardShortcuts: () => void;
    onToggleHidden: () => void;
    /** An item of the Files menu was chosen; it does what its key does. */
    onFiles: (action: FilesChoice) => void;
    /** The search box: it opens the Find dialog, as Alt+F7 does. */
    onFind: () => void;
    /** "⇆ Compare": the two folders are marked for their differences, as Files ▸ Compare folders does. */
    onCompare: () => void;
    /** "Sync": the Synchronize dialog, as Files ▸ Synchronize folders… opens it. */
    onSync: () => void;
  } = $props();

  /** The menus that hold nothing yet. */
  const MENUS = ['Mark', 'Commands', 'Net'] as const;
  const THEME_LABEL = { system: 'System', light: 'Light', dark: 'Dark' } as const;

  type Popup = 'files' | 'view' | 'settings';

  /** The popup that is open; opening one closes the other. */
  let open = $state<Popup | undefined>();

  const toggle = (popup: Popup) => (open = open === popup ? undefined : popup);

  function checkForUpdates() {
    open = undefined;
    onCheckForUpdates();
  }

  function editSettings() {
    open = undefined;
    onEditSettings();
  }

  function chooseFile(action: FilesChoice) {
    open = undefined;
    onFiles(action);
  }

  function chooseLayout(choice: Layout) {
    open = undefined;
    onLayout(choice);
  }

  function closeOnOutside(event: MouseEvent) {
    if (open && !(event.target as Element).closest('.has-popup')) {
      open = undefined;
    }
  }

  /** Escape closes the popup, and so does a shortcut: its action (a dialog, a pane change) takes over. */
  function closeOnKey(event: KeyboardEvent) {
    if (open && (event.key === 'Escape' || keymap.shortcutFor(event) !== undefined)) {
      // The Escape that closes the popup is used up (it does not also stop a walk); a shortcut still acts.
      if (event.key === 'Escape') {
        event.preventDefault();
      }
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

{#snippet viewMenu()}
  <ViewMenu {layout} onChoose={chooseLayout} />
{/snippet}

{#snippet settings()}
  <div class="popup" role="menu">
    <button class="popup-item" type="button" role="menuitem" onclick={() => ((open = undefined), onSettings())}>Settings…</button>
    <button class="popup-item" type="button" role="menuitem" onclick={editSettings}>Edit Settings…</button>
    <button class="popup-item" type="button" role="menuitem" onclick={() => ((open = undefined), onKeyboardShortcuts())}>Keyboard Shortcuts…</button>
    {#if !nativeMenu}
      <button class="popup-item" type="button" role="menuitem" onclick={checkForUpdates}>Check for Updates…</button>
    {/if}
  </div>
{/snippet}

<header class="top">
  <div class="brand">
    <img src="/favicon.svg" alt="" width="22" height="13" />
    <h1>Pairpane</h1>
    {#if version}
      <span class="version">v{version}</span>
    {:else if failure}
      <span class="failure" role="alert">Could not load app info: {displayName(failure)}</span>
    {/if}
  </div>

  <nav class="menu" aria-label="Main menu">
    {@render popup('files', 'Files', files)}
    {#each MENUS as name (name)}
      <button class="placeholder" type="button">{name}</button>
    {/each}
    {@render popup('view', 'View', viewMenu)}
    {@render popup('settings', 'Settings', settings)}
  </nav>

  <button class="search" type="button" aria-keyshortcuts="Alt+F7" onclick={onFind}>
    <span>Find files…</span>
    <kbd aria-hidden="true">Alt F7</kbd>
  </button>

  <div class="tools">
    <button type="button" onclick={onCompare}>⇆ Compare</button>
    <button type="button" onclick={onSync}>⇅ Sync</button>
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

  /* The menus that hold nothing yet give way first; Files, View and Settings stay at every width. */
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
