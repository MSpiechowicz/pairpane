<script lang="ts">
  import { onMount } from 'svelte';

  import { ipc } from '$lib/ipc/client';
  import type { AppInfo } from '$lib/ipc/bindings/AppInfo';
  import ConflictDialog from '$lib/ops/ConflictDialog.svelte';
  import DeleteDialog from '$lib/ops/DeleteDialog.svelte';
  import { FileActions } from '$lib/ops/fileActions.svelte';
  import OpDialog from '$lib/ops/OpDialog.svelte';
  import { Transfers } from '$lib/ops/transfers.svelte';
  import TransfersChip from '$lib/ops/TransfersChip.svelte';
  import TransfersPanel from '$lib/ops/TransfersPanel.svelte';
  import ToolDialogs from '$lib/ops/ToolDialogs.svelte';
  import { focusActiveList } from '$lib/panes/focus';
  import { Hotlist } from '$lib/panes/hotlist.svelte';
  import { PaneTabs } from '$lib/panes/tabs.svelte';
  import { Volumes } from '$lib/panes/volumes.svelte';
  import Dock from '$lib/shell/Dock.svelte';
  import type { Menu } from '$lib/shell/menu';
  import Pane from '$lib/shell/Pane.svelte';
  import { Session } from '$lib/shell/session.svelte';
  import { isTyping, reloadsPage, shortcutFor } from '$lib/shell/shortcuts';
  import Splash from '$lib/shell/Splash.svelte';
  import { Theme } from '$lib/shell/theme.svelte';
  import Toast from '$lib/shell/Toast.svelte';
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

  const transfers = new Transfers(ipc);
  let panelOpen = $state(false);
  const files = new FileActions(ipc, panes, () => active);
  const question = $derived(transfers.question);
  /** A dialog has the keyboard: the shortcuts and Tab are its own then. */
  const modal = $derived(files.modal || question !== undefined);

  function onDockKey(key: string) {
    if (key === 'F5') {
      files.run('copy');
    } else if (key === 'F6') {
      files.run('move');
    } else if (key === 'F7') {
      files.run('newFolder');
    } else if (key === 'F8') {
      files.run('trash');
    }
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

    // Delete is a key of the menus' lists too (Shift+Delete removes a drive from the list, say): not a file's.
    const removing = action === 'trash' || action === 'deletePermanently';
    if (removing && event.target instanceof Element && event.target.closest('[data-menu]')) {
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
      case 'copy':
      case 'move':
      case 'rename':
      case 'renameInline':
      case 'newFolder':
      case 'newFile':
      case 'trash':
      case 'deletePermanently':
      case 'properties':
        files.run(action);
        break;
      case 'transfers':
        panelOpen = !panelOpen;
        break;
    }

    event.preventDefault();
  }

  function onKeyDown(event: KeyboardEvent) {
    const claimed = event.defaultPrevented || modal;
    if (reloadsPage(event)) {
      event.preventDefault(); // even in a text field: a reload would lose the panes and every job on screen
    }

    if (!claimed) {
      switchPane(event);
      runShortcut(event);
    }
  }

  onMount(async () => {
    // The panes open the folders of the last session (the home folder when there is none); the rest does not wait.
    void session.restore();
    void volumes.refresh();
    // Without the desktop runtime there are no jobs.
    void transfers.subscribe().catch(() => {});

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
    onFiles={(action) => files.run(action)}
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
        rename={files.rename}
        onActivate={() => (active = side)}
        onMenu={(kind) => (menu = kind === undefined ? undefined : { side, kind })}
      />
    {/each}
  </main>

  <Dock onKey={onDockKey}>
    <TransfersChip {transfers} open={panelOpen} onToggle={() => (panelOpen = !panelOpen)} />
  </Dock>
</div>

<!-- Above the dock, on the right: the update card, the notice and the transfers stack, so none covers another. -->
<div class="floats">
  <UpdateCard {updates} />
  {#if files.toast}
    {@const toast = files.toast}
    {#key toast.id}
      <Toast
        text={toast.text}
        error={toast.error}
        undoable={toast.undo !== null}
        onUndo={() => void files.undo()}
        onClose={() => files.dismissToast()}
      />
    {/key}
  {/if}
  {#if panelOpen}
    <TransfersPanel {transfers} onClose={() => (panelOpen = false)} />
  {/if}
</div>

{#if files.dialog}
  <OpDialog
    request={files.dialog.request}
    {ipc}
    onClose={() => files.closeDialog()}
    onDone={(name) => files.dialogDone(name)}
    onNotice={(notice) => files.say(notice.text, notice.error)}
  />
{/if}

{#if files.tools.dialog}
  <ToolDialogs {ipc} dialog={files.tools.dialog} onClose={() => files.closeTool()} onDone={(output) => files.toolDone(output)} />
{/if}

{#if files.removal}
  <DeleteDialog
    permanent={files.removal.permanent}
    names={files.removal.names}
    onConfirm={() => files.confirmRemoval()}
    onClose={() => files.closeRemoval()}
  />
{/if}

{#if question}
  {#key `${question.info.job}:${question.info.question}`}
    <ConflictDialog {transfers} {question} />
  {/key}
{/if}

<Splash onEnd={focusActiveList} />

<style>
  .shell {
    display: grid;
    grid-template-rows: 44px minmax(0, 1fr) 44px;
    grid-template-columns: minmax(0, 1fr);
    height: 100%;
  }

  .floats {
    position: fixed;
    right: 16px;
    bottom: 56px;
    z-index: 30;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
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
