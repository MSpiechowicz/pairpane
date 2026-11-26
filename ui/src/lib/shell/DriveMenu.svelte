<script lang="ts">
  import { volumeOf } from '$lib/panes/crumbs';
  import { displayName, formatFree } from '$lib/panes/format';
  import type { PaneTabs } from '$lib/panes/tabs.svelte';
  import type { Volumes } from '$lib/panes/volumes.svelte';
  import type { Remote } from '$lib/remote/remote.svelte';

  import Dropdown from './Dropdown.svelte';

  let {
    tabs,
    volumes,
    remote,
    onClose,
  }: {
    tabs: PaneTabs;
    volumes: Volumes;
    /** Connects to and leaves servers; without it the menu offers neither. */
    remote?: Remote;
    onClose: (refocus: boolean) => void;
  } = $props();

  const listing = $derived(tabs.view.listing);
  const places = $derived(volumes.list);
  /** The volume the pane is in; the menu opens on it. */
  // A server's folders are not on any drive of this computer.
  const current = $derived(listing.remote === undefined ? volumeOf(listing.crumbs, places) : undefined);

  let moved = $state<number>();
  const active = $derived(moved ?? Math.max(0, places.findIndex((volume) => volume === current)));

  // The list is read again every 2 s for as long as the menu is open.
  $effect(() => volumes.poll());

  /** The volume's root opens in the current tab like any other move, so it is in the tab's history. */
  function choose(index: number) {
    void listing.open(places[index].path);
    onClose(true);
  }

  /** The dialog or the disconnect is the pane's (and the app's), not the menu's: the menu closes at once. */
  function connect() {
    onClose(false);
    void remote?.show();
  }

  function disconnect() {
    onClose(true);
    void remote?.disconnect(tabs);
  }
</script>

<Dropdown
  label="Drives"
  count={places.length}
  bind:active={() => active, (index) => (moved = index)}
  selected={(index) => places[index].path === current?.path}
  empty="No drives found."
  onChoose={choose}
  {onClose}
>
  {#snippet row(index)}
    {@const volume = places[index]}
    <span class="fit">{displayName(volume.name)}</span>
    {#if volume.label}<span class="dim fit">{displayName(volume.label)}</span>{/if}
    {#if volume.free !== null}<span class="dim end">{formatFree(volume.free)}</span>{/if}
  {/snippet}

  {#snippet footer()}
    {#if remote}
      <button class="action" type="button" onclick={connect}>Connect to SFTP server…</button>
      {#if listing.remote !== undefined}
        <button class="action" type="button" onclick={disconnect}>Disconnect from {displayName(listing.remote.label)}</button>
      {/if}
    {/if}
  {/snippet}
</Dropdown>
