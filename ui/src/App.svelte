<script lang="ts">
  import { onMount } from 'svelte';

  import { Editor } from '$lib/editor/editor.svelte';
  import EditorOverlay from '$lib/editor/EditorOverlay.svelte';
  import type { AppInfo } from '$lib/ipc/bindings/AppInfo';
  import { ipc as backend } from '$lib/ipc/client';
  import { IpcFailure } from '$lib/ipc/transport';
  import ConflictDialog from '$lib/ops/ConflictDialog.svelte';
  import DeleteDialog from '$lib/ops/DeleteDialog.svelte';
  import { FileActions } from '$lib/ops/fileActions.svelte';
  import OpDialog from '$lib/ops/OpDialog.svelte';
  import PasswordDialog from '$lib/ops/PasswordDialog.svelte';
  import { Passwords } from '$lib/ops/passwords.svelte';
  import { Transfers } from '$lib/ops/transfers.svelte';
  import TransfersChip from '$lib/ops/TransfersChip.svelte';
  import TransfersPanel from '$lib/ops/TransfersPanel.svelte';
  import ToolDialogs from '$lib/ops/ToolDialogs.svelte';
  import { focusActiveList } from '$lib/panes/focus';
  import { sameArchivePath } from '$lib/panes/archive';
  import { Hotlist } from '$lib/panes/hotlist.svelte';
  import { PaneTabs } from '$lib/panes/tabs.svelte';
  import { Volumes } from '$lib/panes/volumes.svelte';
  import Dock from '$lib/shell/Dock.svelte';
  import type { FilesChoice, Menu } from '$lib/shell/menu';
  import Pane from '$lib/shell/Pane.svelte';
  import { Session } from '$lib/shell/session.svelte';
  import { isTyping, reloadsPage, shortcutFor } from '$lib/shell/shortcuts';
  import Splash from '$lib/shell/Splash.svelte';
  import { Theme } from '$lib/shell/theme.svelte';
  import Toast from '$lib/shell/Toast.svelte';
  import TopBar from '$lib/shell/TopBar.svelte';
  import UpdateCard from '$lib/updates/UpdateCard.svelte';
  import { Updates } from '$lib/updates/updates.svelte';
  import QuickView from '$lib/viewer/QuickView.svelte';
  import { Viewer } from '$lib/viewer/viewer.svelte';
  import ViewerOverlay from '$lib/viewer/ViewerOverlay.svelte';

  type Side = 'left' | 'right';

  let info = $state<AppInfo | undefined>();
  let failure = $state<string | undefined>();
  let active = $state<Side>('left');
  let showHidden = $state(false);
  /** The dropdown that is open, and the pane under whose header it is. */
  let menu = $state<{ side: Side; kind: Menu } | undefined>();

  // Whatever reads inside an archive that needs a password asks for it, then goes on: the panes, the jobs and the
  // viewer all work through this transport.
  const passwords = new Passwords(backend);
  const ipc = passwords.wrap(backend);
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

  /** An add to an archive ended (or stopped): the panes that show that archive list it again, from the file. */
  function archiveChanged(archive: string) {
    for (const tabs of [panes.left, panes.right]) {
      const { listing } = tabs.view;
      if (listing.archive !== undefined && sameArchivePath(listing.archive.archive, archive)) {
        void listing.reload();
      }
    }
  }

  const transfers = new Transfers(ipc, (job) => {
    if (job.kind === 'addToArchive') {
      archiveChanged(job.to);
    }
  });
  let panelOpen = $state(false);
  const files = new FileActions(ipc, panes, () => active);
  const viewer = new Viewer(ipc, panes, () => active);
  const editor = new Editor(ipc, {
    under: () => viewer.under,
    say: (text, error) => files.say(text, error),
    view: (target, notice) => viewer.openAt(target, notice),
  });
  const question = $derived(transfers.question);
  /** A dialog (or the viewer's or editor's overlay) has the keyboard: the shortcuts and Tab are its own then. */
  const modal = $derived(files.modal || question !== undefined || viewer.modal || editor.modal || passwords.modal);

  /** F4 and the Files menu's editor: an archive's entries are read-only, so there is nothing to edit there. */
  function edit(builtin = false) {
    if (!files.refuseEdit()) {
      editor.edit(builtin);
    }
  }

  function onDockKey(key: string) {
    if (key === 'F3') {
      viewer.open();
    } else if (key === 'F4') {
      edit();
    } else if (key === 'F5') {
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
  // A restart cannot be stopped once it has begun: an editor with changes asks before it.
  const updates = new Updates(
    ipc,
    () => info?.version,
    () => editor.confirmDiscard(),
  );
  // macOS has a native app menu (with "Check for Updates…"); Windows and Linux have only the top bar.
  const nativeMenu = /Mac/.test(navigator.userAgent);

  /**
   * Tab switches the active pane while focus is in a pane's list or nowhere (the page itself). In the top bar, the
   * menus (a pane's dropdowns too), the dock and a pane's text fields (the mask prompt) it moves focus as usual,
   * so every control stays reachable from the keyboard. While the quick view is open there is one pane to work in.
   */
  function switchPane(event: KeyboardEvent) {
    const target = event.target;
    const inPane = target === document.body || (target instanceof Element && target.closest('[data-pane]'));
    const inMenu = target instanceof Element && target.closest('[data-menu]') !== null;
    if (
      viewer.quick ||
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
      case 'extract':
      case 'pack':
        files.run(action);
        break;
      case 'transfers':
        panelOpen = !panelOpen;
        break;
      case 'view':
        viewer.open();
        break;
      case 'edit':
        edit();
        break;
      case 'quickView':
        viewer.toggleQuick();
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

  function chooseFile(choice: FilesChoice) {
    if (choice === 'editBuiltin') {
      edit(true);
    } else {
      files.run(choice);
    }
  }

  function editSettings() {
    ipc.invoke('settings_open', undefined).catch((error) => files.say(IpcFailure.from(error).message, true));
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
      // The native menu's "Check for Updates…" (macOS) and a window close that would lose changes arrive here.
      await ipc.invoke('menu_subscribe', {
        onCommand: (command) => {
          if (command === 'checkForUpdates') {
            void updates.check(true);
          } else if (command === 'closeRequested') {
            void editor.onCloseRequested();
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
    onEditSettings={editSettings}
    onToggleHidden={toggleHidden}
    onFiles={chooseFile}
  />

  <main class="panes">
    {#each ['left', 'right'] as const as side (side)}
      {#if viewer.quick && side !== active}
        <QuickView {viewer} />
      {:else}
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
      {/if}
    {/each}
  </main>

  <Dock inArchive={panes[active].view.listing.readonly} onKey={onDockKey}>
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

{#if passwords.prompt}
  {#key passwords.prompt.archive}
    <PasswordDialog {passwords} prompt={passwords.prompt} />
  {/key}
{/if}

{#if viewer.overlay}
  <ViewerOverlay session={viewer.overlay} onClose={() => viewer.close()} />
{/if}

{#if editor.session}
  <EditorOverlay {editor} session={editor.session} />
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
