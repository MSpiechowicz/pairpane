<script lang="ts">
  import { onMount } from 'svelte';

  import { ipc } from '$lib/ipc/client';
  import type { AppInfo } from '$lib/ipc/bindings/AppInfo';
  import { Hotlist } from '$lib/panes/hotlist.svelte';
  import { PaneTabs } from '$lib/panes/tabs.svelte';
  import { Volumes } from '$lib/panes/volumes.svelte';
  import Dock from '$lib/shell/Dock.svelte';
  import type { Menu } from '$lib/shell/menu';
  import Pane from '$lib/shell/Pane.svelte';
  import { Session } from '$lib/shell/session.svelte';
  import { isTyping, shortcutFor } from '$lib/shell/shortcuts';
  import Splash from '$lib/shell/Splash.svelte';
  import { Theme } from '$lib/shell/theme.svelte';
  import TopBar from '$lib/shell/TopBar.svelte';
  import UpdateCard from '$lib/updates/UpdateCard.svelte';
  import { Updates } from '$lib/updates/updates.svelte';

  type Side = 'left' | 'right';

  let info = $state<AppInfo | undefined>();
  let failure = $state<string | undefined>();
  let active = $state<Side>('left');
  let showHidden = $state(false);
  /** The dropdown that is open, and the pane under whose header it is. */
  let menu = $state<{ side: Side; kind: Menu } | undefined>();

  const panes = { left: new PaneTabs(ipc, 'left'), right: new PaneTabs(ipc, 'right') };
  // The volumes name the first step of the paths and fill the drive menu.
  const volumes = new Volumes(ipc);
  const hotlist = new Hotlist();
  const session = new Session(ipc, {
    panes,
    hotlist,
    active: { get: () => active, set: (side) => (active = side) },
  });
  session.watch();

  function toggleHidden() {
    showHidden = !showHidden;
    void panes.left.view.listing.setShowHidden(showHidden);
    void panes.right.view.listing.setShowHidden(showHidden);
  }

  const theme = new Theme();
  const updates = new Updates(ipc, () => info?.version);
  // macOS has a native app menu (with "Check for Updates…"); Windows and Linux have only the top bar.
  const nativeMenu = /Mac/.test(navigator.userAgent);

  /**
   * Tab switches the active pane while focus is in a pane's list or nowhere (the page itself). In the top bar, the
   * menus (a pane's dropdowns too), the dock and a pane's text fields (the mask prompt) it moves focus as usual,
   * so every control stays reachable from the keyboard.
   */
  function switchPane(event: KeyboardEvent) {
    const target = event.target;
    const inPane = target === document.body || (target instanceof Element && target.closest('[data-pane]'));
    const inMenu = target instanceof Element && target.closest('[data-menu]') !== null;
    if (
      event.key !== 'Tab' ||
      event.shiftKey ||
      event.ctrlKey ||
      event.altKey ||
      event.metaKey ||
      !inPane ||
      inMenu ||
      isTyping(target)
    ) {
      return;
    }

    event.preventDefault();
    active = active === 'left' ? 'right' : 'left';
  }

  /** Opens a dropdown under a pane's header; that pane becomes the active one. */
  function showMenu(side: Side, kind: Menu) {
    active = side;
    menu = { side, kind };
  }

  /**
   * Tab, history and menu shortcuts act on the active pane. Alt+Left is the webview's own "go back", so a handled
   * shortcut always prevents the default.
   */
  function runShortcut(event: KeyboardEvent) {
    const action = shortcutFor(event);
    if (action === undefined || isTyping(event.target)) {
      return;
    }

    const tabs = panes[active];
    switch (action) {
      case 'newTab':
        tabs.add();
        break;
      case 'closeTab':
        tabs.close();
        break;
      case 'nextTab':
        tabs.cycle(1);
        break;
      case 'previousTab':
        tabs.cycle(-1);
        break;
      case 'back':
        tabs.back();
        break;
      case 'forward':
        tabs.forward();
        break;
      case 'driveMenuLeft':
        showMenu('left', 'drives');
        break;
      case 'driveMenuRight':
        showMenu('right', 'drives');
        break;
      case 'hotlist':
        showMenu(active, 'hotlist');
        break;
    }

    event.preventDefault();
  }

  function onKeyDown(event: KeyboardEvent) {
    if (!event.defaultPrevented) {
      switchPane(event);
      runShortcut(event);
    }
  }

  /** The splash was above the app: the focus goes where the app keeps it, the active pane's list. */
  function focusActiveList() {
    if (document.activeElement === null || document.activeElement === document.body) {
      document.querySelector<HTMLElement>('[data-pane][aria-current="true"] [role="listbox"]')?.focus();
    }
  }

  onMount(async () => {
    // The panes open the folders of the last session (the home folder when there is none); the rest does not wait.
    void session.restore();
    void volumes.refresh();

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

<svelte:window onkeydown={onKeyDown} />

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
    {#each ['left', 'right'] as const as side (side)}
      <Pane
        label={side === 'left' ? 'Left pane' : 'Right pane'}
        tabs={panes[side]}
        {volumes}
        {hotlist}
        menu={menu?.side === side ? menu.kind : undefined}
        active={active === side}
        onActivate={() => (active = side)}
        onMenu={(kind) => (menu = kind === undefined ? undefined : { side, kind })}
      />
    {/each}
  </main>

  <Dock />
</div>

<UpdateCard {updates} />

<Splash onEnd={focusActiveList} />

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
