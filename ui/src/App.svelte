<script lang="ts">
  import { onMount } from 'svelte';

  import { ipc } from '$lib/ipc/client';
  import type { AppInfo } from '$lib/ipc/bindings/AppInfo';
  import { Listing } from '$lib/panes/listing.svelte';
  import Dock from '$lib/shell/Dock.svelte';
  import Pane from '$lib/shell/Pane.svelte';
  import { Theme } from '$lib/shell/theme.svelte';
  import TopBar from '$lib/shell/TopBar.svelte';
  import UpdateCard from '$lib/updates/UpdateCard.svelte';
  import { Updates } from '$lib/updates/updates.svelte';

  type Side = 'left' | 'right';

  let info = $state<AppInfo | undefined>();
  let failure = $state<string | undefined>();
  let active = $state<Side>('left');
  let showHidden = $state(false);

  const panes = { left: new Listing(ipc, 'left'), right: new Listing(ipc, 'right') };

  function toggleHidden() {
    showHidden = !showHidden;
    void panes.left.setShowHidden(showHidden);
    void panes.right.setShowHidden(showHidden);
  }

  const theme = new Theme();
  const updates = new Updates(ipc, () => info?.version);
  // macOS has a native app menu (with "Check for Updates…"); Windows and Linux have only the top bar.
  const nativeMenu = /Mac/.test(navigator.userAgent);

  /**
   * Tab switches the active pane while focus is in a pane or nowhere (the page itself). In the top bar, the menus
   * and the dock it moves focus as usual, so every control stays reachable from the keyboard.
   */
  function switchPane(event: KeyboardEvent) {
    const target = event.target;
    const inPane = target === document.body || (target instanceof Element && target.closest('[data-pane]'));
    if (event.key !== 'Tab' || event.shiftKey || event.ctrlKey || event.altKey || event.metaKey || !inPane) {
      return;
    }

    event.preventDefault();
    active = active === 'left' ? 'right' : 'left';
  }

  onMount(async () => {
    // Both panes start in the home folder.
    void panes.left.open(null);
    void panes.right.open(null);

    try {
      info = await ipc.invoke('app_info', undefined);
    } catch (error) {
      failure = error instanceof Error ? error.message : String(error);
    }

    try {
      // The native menu's "Check for Updates…" (macOS) arrives here.
      await ipc.invoke('menu_subscribe', {
        onCommand: (command) => {
          if (command === 'checkForUpdates') {
            void updates.check(true);
          }
        },
      });
    } catch {
      // Without the desktop runtime there is no menu.
    }

    // Silent unless a newer release exists.
    void updates.check(false);
  });
</script>

<svelte:window onkeydown={switchPane} />

<div class="shell">
  <TopBar
    version={info?.version}
    {failure}
    {theme}
    {nativeMenu}
    {showHidden}
    onCheckForUpdates={() => void updates.check(true)}
    onToggleHidden={toggleHidden}
  />

  <main class="panes">
    <Pane label="Left pane" listing={panes.left} active={active === 'left'} onActivate={() => (active = 'left')} />
    <Pane label="Right pane" listing={panes.right} active={active === 'right'} onActivate={() => (active = 'right')} />
  </main>

  <Dock />
</div>

<UpdateCard {updates} />

<style>
  .shell {
    display: grid;
    grid-template-rows: 44px minmax(0, 1fr) 44px;
    grid-template-columns: minmax(0, 1fr);
    height: 100%;
  }

  .panes {
    display: flex;
    gap: 6px;
    min-height: 0;
    padding: 6px;
  }

  @media (max-width: 760px) {
    .panes {
      flex-direction: column;
    }
  }
</style>
