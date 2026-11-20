<script lang="ts">
  import { volumeOf } from '$lib/panes/crumbs';
  import { formatFree } from '$lib/panes/format';
  import type { PaneTabs } from '$lib/panes/tabs.svelte';
  import type { Volumes } from '$lib/panes/volumes.svelte';

  import Dropdown from './Dropdown.svelte';

  let {
    tabs,
    volumes,
    onClose,
  }: {
    tabs: PaneTabs;
    volumes: Volumes;
    onClose: (refocus: boolean) => void;
  } = $props();

  const listing = $derived(tabs.view.listing);
  const places = $derived(volumes.list);
  /** The volume the pane is in; the menu opens on it. */
  const current = $derived(volumeOf(listing.crumbs, places));

  let moved = $state<number>();
  const active = $derived(moved ?? Math.max(0, places.findIndex((volume) => volume === current)));

  // The list is read again every 2 s for as long as the menu is open.
  $effect(() => volumes.poll());

  /** The volume's root opens in the current tab like any other move, so it is in the tab's history. */
  function choose(index: number) {
    void listing.open(places[index].path);
    onClose(true);
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
    <span class="fit">{volume.name}</span>
    {#if volume.label}<span class="dim fit">{volume.label}</span>{/if}
    {#if volume.free !== null}<span class="dim end">{formatFree(volume.free)}</span>{/if}
  {/snippet}
</Dropdown>
