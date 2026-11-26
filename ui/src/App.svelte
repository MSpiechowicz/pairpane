<script lang="ts">
  import { onMount } from 'svelte';

  import CompareTools from '$lib/compare/CompareTools.svelte';
  import { Comparison } from '$lib/compare/controller.svelte';
  import { FileDiff } from '$lib/diff/fileDiff.svelte';
  import DiffOverlay from '$lib/diff/DiffOverlay.svelte';
  import { pickFiles } from '$lib/diff/pick';
  import { Editor } from '$lib/editor/editor.svelte';
  import EditorOverlay from '$lib/editor/EditorOverlay.svelte';
  import { Unsaved } from '$lib/editor/unsaved';
  import FindDialog from '$lib/find/FindDialog.svelte';
  import { Finder } from '$lib/find/finder.svelte';
  import { goTo } from '$lib/find/goto';
  import { SavedSearches } from '$lib/find/saved.svelte';
  import type { AppInfo } from '$lib/ipc/bindings/AppInfo';
  import { ipc as backend } from '$lib/ipc/client';
  import { IpcFailure } from '$lib/ipc/transport';
  import FileDialogs from '$lib/ops/FileDialogs.svelte';
  import { FileActions } from '$lib/ops/fileActions.svelte';
  import { Passwords } from '$lib/ops/passwords.svelte';
  import { QuitGuard } from '$lib/ops/quit.svelte';
  import { REMOTE_NOTICE } from '$lib/ops/targets';
  import { Transfers } from '$lib/ops/transfers.svelte';
  import TransfersChip from '$lib/ops/TransfersChip.svelte';
  import TransfersPanel from '$lib/ops/TransfersPanel.svelte';
  import { focusActiveList } from '$lib/panes/focus';
  import { reloadShowing } from '$lib/panes/archive';
  import { FlatView } from '$lib/panes/flat.svelte';
  import { followPreferences } from '$lib/panes/hidden.svelte';
  import { Hotlist } from '$lib/panes/hotlist.svelte';
  import { PaneTabs } from '$lib/panes/tabs.svelte';
  import { Volumes } from '$lib/panes/volumes.svelte';
  import { Remote } from '$lib/remote/remote.svelte';
  import { reloadRemote } from '$lib/remote/transfer';
  import { SavedRenames } from '$lib/rename/saved.svelte';
  import { CommandLine, placeOf } from '$lib/shell/command-line.svelte';
  import Dock from '$lib/shell/Dock.svelte';
  import type { FilesChoice, Menu } from '$lib/shell/menu';
  import Pane from '$lib/shell/Pane.svelte';
  import { preferences } from '$lib/shell/preferences.svelte';
  import { Session } from '$lib/shell/session.svelte';
  import ShellDialogs from '$lib/shell/ShellDialogs.svelte';
  import { DIALOG_OF, opensDialog, type ShellDialog } from '$lib/shell/dialogs';
  import { isTyping, reloadsPage } from '$lib/shell/shortcuts';
  import { keymap } from '$lib/shell/keymap.svelte';
  import { switchesPane } from '$lib/shell/switchPane';
  import { type ActionContext, runAction } from '$lib/shell/actions';
  import { isAction } from '$lib/palette/commands';
  import Palette from '$lib/palette/Palette.svelte';
  import { namesOf, recentFolders } from '$lib/palette/sources';
  import { Palette as PaletteModel, type PaletteItem } from '$lib/palette/model.svelte';
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
  /** The dialog of the top bar's Settings menu that is open. */
  let dialog = $state<ShellDialog>();
  /** The dropdown that is open, and the pane under whose header it is. */
  let menu = $state<{ side: Side; kind: Menu } | undefined>();

  // Whatever reads inside an archive that needs a password asks for it, then goes on: the panes, the jobs and the
  // viewer all work through this transport.
  const passwords = new Passwords(backend);
  const ipc = passwords.wrap(backend);
  const hooks = { showHidden: () => preferences.value.showHidden };
  const panes = { left: new PaneTabs(ipc, 'left', hooks), right: new PaneTabs(ipc, 'right', hooks) };
  // The volumes name the first step of the paths and fill the drive menu.
  const volumes = new Volumes(ipc);
  const hotlist = new Hotlist();
  const searches = new SavedSearches();
  const renames = new SavedRenames();
  const finder = new Finder(ipc, (id) =>
    [panes.left, panes.right].some((tabs) => tabs.view.listing.results === id),
  );
  const session = new Session(ipc, {
    panes,
    hotlist,
    searches,
    renames,
    keymap,
    preferences,
    active: { get: () => active, set: (side) => (active = side) },
  });
  session.watch();
  followPreferences(panes);

  const palette = new PaletteModel({
    folders: () => recentFolders([panes[active], panes[active === 'left' ? 'right' : 'left']]),
    names: () => namesOf(panes[active].view),
  });

  const toggleHidden = () => preferences.set({ showHidden: !preferences.value.showHidden });
  const exit = () => void ipc.invoke('app_quit', undefined).catch((error) => files.say(IpcFailure.from(error).message, true));

  const transfers = new Transfers(ipc, (job, result) => {
    if (job.kind === 'addToArchive') {
      reloadShowing([panes.left, panes.right], job.to);
    }

    reloadRemote([panes.left, panes.right], job.remote);
    files.renames.ended(job, result);
    files.syncs.ended(job, result);
  });
  let panelOpen = $state(false);
  const files = new FileActions(ipc, panes, () => active);
  // Connecting to a server opens its home folder in the active pane.
  const remote = new Remote(ipc, panes, () => active, (text, error) => files.say(text, error));
  const flat = new FlatView(ipc, panes, (text, error) => files.say(text, error), () => preferences.value.showHidden);
  flat.watch();
  const comparison = new Comparison(ipc, panes, (text, error) => files.say(text, error), () => preferences.value.showHidden);
  comparison.syncer.onQueued = (job, undoable) => files.syncs.expect(job, undoable);
  const viewer = new Viewer(ipc, panes, () => active);
  // The editor and the diff both hold text that is not saved: the backend hears of the two as one.
  const unsaved = new Unsaved(ipc);
  const editor = new Editor(
    ipc,
    {
      under: () => viewer.under,
      say: (text, error) => files.say(text, error),
      view: (target, notice) => viewer.openAt(target, notice),
      blocked: (): boolean => diff.busy,
    },
    unsaved,
  );
  const diff = new FileDiff(ipc, unsaved, {
    pick: () =>
      pickFiles({ active: panes[active].view, other: panes[active === 'left' ? 'right' : 'left'].view, side: active, quick: viewer.quick }),
    say: (text, error) => files.say(text, error),
    blocked: (): boolean => editor.busy,
  });
  // Quitting with transfers running is asked about too (unless the settings say not to), after the unsaved text.
  const quit = new QuitGuard(unsaved, transfers);
  quit.watch();
  const question = $derived(transfers.question);
  /** A dialog (or the viewer's or editor's overlay) has the keyboard: the shortcuts and Tab are its own then. */
  const modal = $derived(
    files.modal ||
      question !== undefined ||
      viewer.modal ||
      editor.modal ||
      diff.modal ||
      passwords.modal ||
      remote.modal ||
      finder.modal ||
      comparison.modal ||
      palette.shown ||
      dialog !== undefined ||
      quit.asking,
  );

  /** Alt+F7: the Find dialog, on the folder the active pane shows (inside an archive: the folder that holds it). */
  function find() {
    const { folder, remote: server } = panes[active].view.listing;
    if (server !== undefined) {
      files.say(REMOTE_NOTICE, true);
    } else if (folder === undefined) {
      files.say('Wait until the folder has been listed.', true);
    } else {
      finder.show(folder);
    }
  }

  /** F3 and the Files menu's viewer: a file on a server has no path on disk to show yet, so a card says so. */
  function view() {
    if (panes[active].view.listing.remote !== undefined) {
      files.say(REMOTE_NOTICE, true);
    } else {
      viewer.open();
    }
  }

  /** F4 and the Files menu's editor: an archive's entries are read-only, so there is nothing to edit there. */
  function edit(builtin = false) {
    if (!files.refuseEdit()) {
      editor.edit(builtin);
    }
  }

  const theme = new Theme();
  theme.watch();
  // A restart cannot be stopped once it has begun: an editor or a diff with changes, or transfers, ask before it.
  const updates = new Updates(
    ipc,
    () => info?.version,
    async () => (await editor.confirmDiscard()) && (await diff.confirmDiscard()) && (await quit.discard()) === 'go',
    () => preferences.value.checkUpdatesAtStart,
  );
  // macOS has a native app menu (with "Check for Updates…"); Windows and Linux have only the top bar.
  const nativeMenu = /Mac/.test(navigator.userAgent);

  /** Tab switches the active pane (see {@link switchesPane}). */
  function switchPane(event: KeyboardEvent) {
    if (switchesPane(event, viewer.quick)) {
      event.preventDefault();
      active = active === 'left' ? 'right' : 'left';
    }
  }

  /** Opens a dropdown under a pane's header; that pane becomes the active one. */
  function showMenu(side: Side, kind: Menu) {
    active = side;
    menu = { side, kind };
  }

  /** What the actions of the keymap and the palette act on (see `runAction`). */
  const commandLine = new CommandLine(ipc, () => placeOf(panes[active].view.listing), (text, error) => files.say(text, error));

  const actions: ActionContext = {
    tabs: () => panes[active],
    active: () => active,
    showMenu,
    files: (action) => files.run(action),
    toggleTransfers: () => (panelOpen = !panelOpen),
    view: () => view(),
    edit: () => edit(),
    compareFiles: () => diff.open(),
    quickView: () => viewer.toggleQuick(),
    find: () => find(),
    flatView: () => flat[active].toggle(),
    terminal: () => commandLine.terminal(),
    palette: () => palette.show(),
  };

  /** A shortcut of the keymap. Alt+Left is the webview's own "go back", so a handled shortcut always prevents the default. */
  function runShortcut(event: KeyboardEvent) {
    const action = keymap.shortcutFor(event);
    if (action === undefined || isTyping(event.target)) {
      return;
    }

    // Delete is a key of the menus' lists too (Shift+Delete removes a drive from the list, say): not a file's.
    const removing = action === 'trash' || action === 'deletePermanently';
    if (removing && event.target instanceof Element && event.target.closest('[data-menu]')) {
      return;
    }

    runAction(action, actions);
    event.preventDefault();
  }

  /** Runs what the palette took: a command, a folder opened in the active pane, a name the cursor goes to. */
  function takePicked(item: PaletteItem) {
    if (item.kind === 'folder') {
      void panes[active].view.listing.open(item.path);
    } else if (item.kind === 'file') {
      panes[active].view.pointAt(item.name);
    } else if (item.id === 'toggleHidden') {
      toggleHidden();
    } else if (item.id === 'connectSftp') {
      void remote.show();
    } else if (item.id === 'disconnectSftp') {
      void remote.disconnect(panes[active]);
    } else if (item.id === 'editSettings') {
      editSettings();
    } else if (opensDialog(item.id)) {
      dialog = DIALOG_OF[item.id];
    } else if (item.id === 'quit') {
      exit();
    } else if (item.id === 'checkUpdates') {
      void updates.check(true);
    } else if (isAction(item.id)) {
      runAction(item.id, actions);
    } else {
      chooseFile(item.id);
    }
  }

  function onKeyDown(event: KeyboardEvent) {
    const claimed = event.defaultPrevented || modal;
    if (reloadsPage(event)) {
      event.preventDefault(); // even in a text field: a reload would lose the panes and every job on screen
    }

    if (!claimed) {
      flat.onKey(event, active);
      switchPane(event);
      runShortcut(event);
    }
  }

  function chooseFile(choice: FilesChoice) {
    if (choice === 'editBuiltin') {
      edit(true);
    } else if (choice === 'compareFiles') {
      diff.open();
    } else if (choice === 'find') {
      find();
    } else if (choice === 'compare' || choice === 'synchronize') {
      comparison.run(choice);
    } else {
      files.run(choice);
    }
  }

  function editSettings() {
    ipc.invoke('settings_open', undefined).catch((error) => files.say(IpcFailure.from(error).message, true));
  }

  onMount(async () => {
    // The panes open the folders of the last session (the home folder when there is none); the rest does not wait.
    // The startup check does: what the session restores says whether it is wanted.
    // Not when the state could not be read: whether the user turned the check off is not known then.
    void session.restore().then((known) => (known ? updates.checkAtStart() : undefined));
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
            // Each overlay with changes is asked about, one after the other, and the transfers last.
            void editor.onCloseRequested(async () => quit.after(await diff.discard()));
          }
        },
      });
    } catch {
      // Without the desktop runtime there is no menu.
    }
  });
</script>

<svelte:window onkeydown={onKeyDown} />

<div class="shell">
  <TopBar
    version={info?.version}
    {failure}
    {theme}
    {nativeMenu}
    showHidden={preferences.value.showHidden}
    layout={panes[active].view.layout}
    onLayout={(layout) => (panes[active].view.layout = layout)}
    onCheckForUpdates={() => void updates.check(true)}
    onDialog={(shown) => (dialog = shown)}
    onEditSettings={editSettings}
    onQuit={exit}
    onToggleHidden={toggleHidden}
    onFiles={chooseFile}
    onFind={find}
    onCompare={() => comparison.quick()}
    onSync={() => comparison.synchronize()}
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
          {remote}
          menu={menu?.side === side ? menu.kind : undefined}
          active={active === side}
          rename={files.rename}
          flat={flat[side]}
          onActivate={() => (active = side)}
          onMenu={(kind) => (menu = kind === undefined ? undefined : { side, kind })}
        />
      {/if}
    {/each}
  </main>

  <Dock
    inArchive={panes[active].view.listing.readonly && panes[active].view.listing.remote === undefined}
    inResults={panes[active].view.listing.results !== undefined}
    {commandLine}
    onAction={(action) => runAction(action, actions)}
  >
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

{#if palette.shown}
  <Palette {palette} onTake={takePicked} />
{/if}

<ShellDialogs open={dialog} {ipc} {quit} {remote} onEditSettings={editSettings} onClose={() => (dialog = undefined)} />

<FileDialogs {ipc} {files} {transfers} {passwords} {renames} />

{#if viewer.overlay}
  <ViewerOverlay session={viewer.overlay} onClose={() => viewer.close()} />
{/if}

{#if finder.root !== undefined}
  <FindDialog
    {finder}
    saved={searches}
    results={panes[active].view.listing}
    onGoTo={(hit) => finder.root !== undefined && void goTo(panes[active].view, finder.root, hit)}
  />
{/if}

<CompareTools {comparison} />

{#if editor.session}
  <EditorOverlay {editor} session={editor.session} />
{/if}

<DiffOverlay {diff} />

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
