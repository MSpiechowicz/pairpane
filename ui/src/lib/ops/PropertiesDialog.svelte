<script lang="ts">
  import { onMount } from 'svelte';

  import type { Entry } from '$lib/ipc/bindings/Entry';
  import type { ItemProperties } from '$lib/ipc/bindings/ItemProperties';
  import { IpcFailure, type Transport } from '$lib/ipc/transport';
  import { displayName, formatSize } from '$lib/panes/format';
  import type { FolderSizes } from '$lib/panes/sizes.svelte';
  import Modal from '$lib/shell/Modal.svelte';

  import { what } from './dialogs';
  import DialogFooter from './DialogFooter.svelte';
  import { formatMode, formatStamp } from './jobs';
  import './fields.css';

  let {
    ipc,
    path,
    entry,
    sizes,
    onClose,
  }: {
    ipc: Transport;
    /** The folder the item is in. */
    path: string;
    entry: Entry;
    /** The pane's folder sizes: a folder's size is measured there, as the Size column does. */
    sizes: FolderSizes;
    onClose: () => void;
  } = $props();

  let properties = $state.raw<ItemProperties | undefined>();
  let failure = $state<string | undefined>();

  const KINDS = { dir: 'Folder', file: 'File', other: 'Other' } as const;
  const WINDOWS = [
    ['readOnly', 'Read-only'],
    ['hidden', 'Hidden'],
    ['system', 'System'],
    ['archive', 'Archive'],
  ] as const;

  const time = (millis: number | null) => (millis === null ? 'Unknown' : formatStamp(millis));

  /** `1.3 KB (1,336 bytes)`: the exact count only where the rounded one hides it. */
  const bytes = (size: number) => (size < 1024 ? formatSize(size) : `${formatSize(size)} (${size.toLocaleString('en-US')} bytes)`);

  /** A folder's size is what the pane has measured so far, which may be nothing yet. */
  function sizeText(item: ItemProperties): string {
    if (item.kind !== 'dir') {
      return bytes(item.size);
    }

    const measured = sizes.sizeOf(item.name);
    return measured === undefined ? 'Calculating…' : bytes(measured);
  }

  /** The lines of the dialog; what the system does not have (owner on Windows, a link target) has none. */
  function rowsOf(item: ItemProperties): [string, string][] {
    const rows: [string, string][] = [
      ['Name', item.name],
      ['Type', item.linkTarget === null ? KINDS[item.kind] : `Symbolic link to a ${KINDS[item.kind].toLowerCase()}`],
      ['Location', path],
      ['Size', sizeText(item)],
      ['Created', time(item.created)],
      ['Modified', time(item.modified)],
      ['Accessed', time(item.accessed)],
    ];

    if (item.linkTarget !== null) {
      rows.push(['Points to', item.linkTarget]);
    }

    if (item.mode !== null) {
      rows.push(['Permissions', formatMode(item.mode)]);
    }

    if (item.owner !== null) {
      rows.push(['Owner', item.owner]);
    }

    if (item.group !== null) {
      rows.push(['Group', item.group]);
    }

    const windows = item.windows;
    if (windows !== null) {
      const set = WINDOWS.filter(([flag]) => windows[flag]).map(([, label]) => label);
      rows.push(['Attributes', set.length > 0 ? set.join(', ') : 'None']);
    }

    return rows;
  }

  onMount(() => {
    if (entry.kind === 'dir') {
      sizes.measure(entry.name);
    }

    ipc.invoke('ops_properties', { path, name: entry.name }).then(
      (read) => (properties = read),
      (error) => (failure = IpcFailure.from(error).message),
    );
  });
</script>

<Modal title="Properties of {what([entry.name])}" {onClose}>
  {#if properties}
    <dl>
      {#each rowsOf(properties) as [label, value] (label)}
        <dt>{label}</dt>
        <dd>{displayName(value)}</dd>
      {/each}
    </dl>
  {:else if failure === undefined}
    <p class="hint">Reading…</p>
  {/if}
  <DialogFooter {failure} busy={false} cancel="Close" focus="cancel" onCancel={onClose} />
</Modal>

<style>
  dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 4px 14px;
    margin: 0;
  }

  dt {
    color: var(--dim);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
    user-select: text;
  }
</style>
