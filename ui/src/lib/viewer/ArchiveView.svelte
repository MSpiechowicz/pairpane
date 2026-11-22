<script lang="ts">
  import type { ArchiveListing } from '$lib/ipc/bindings/ArchiveListing';
  import { IpcFailure } from '$lib/ipc/transport';
  import { displayName, formatDate, formatSize } from '$lib/panes/format';

  import type { PreviewProps } from './previews';
  import { takeFocus } from './viewProps.svelte';
  import ViewNotice from './ViewNotice.svelte';

  let { session, focus = false }: PreviewProps = $props();

  /** A folder's name ends in `/` (the backend's, or this one when it did not). */
  const slash = (entry: ArchiveListing['entries'][number]) => (entry.dir && !entry.name.endsWith('/') ? '/' : '');

  let listing = $state.raw<ArchiveListing | undefined>();
  let error = $state<string | undefined>();

  $effect(() => {
    const id = session.id;
    let current = true;

    if (id !== undefined) {
      session.ipc.invoke('view_archive', { session: id }).then(
        (answer) => {
          if (current) {
            listing = answer;
          }
        },
        (reason: unknown) => {
          if (current) {
            error = IpcFailure.from(reason).message;
          }
        },
      );
    }

    return () => {
      current = false;
    };
  });
</script>

<div class="archive">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex (a scrolling region takes the keyboard to be scrolled by it) -->
  <div class="scroll" role="region" aria-label="Contents of {displayName(session.name)}" tabindex="0" {@attach takeFocus(focus)}>
    {#if listing}
      <table>
        <thead>
          <tr><th>Name</th><th class="num">Size</th><th>Modified</th></tr>
        </thead>
        <tbody>
          {#each listing.entries as entry, at (at)}
            <tr class:dir={entry.dir}>
              <td class="name">{displayName(entry.name)}{slash(entry)}</td>
              <td class="num">{entry.dir ? 'Folder' : formatSize(entry.size)}</td>
              <td>{formatDate(entry.modified)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
  <ViewNotice
    {error}
    text={listing?.truncated ? 'Only the first entries are listed' : listing?.entries.length === 0 ? 'The archive is empty.' : undefined}
  />
</div>

<style>
  .archive {
    position: relative;
    flex: 1;
    min-height: 0;
  }

  .scroll {
    height: 100%;
    padding: 4px 0 40px;
    overflow: auto;
    outline: none;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }

  th {
    position: sticky;
    top: 0;
    background: var(--panel);
    color: var(--dim);
    font-weight: 500;
    text-align: left;
  }

  th,
  td {
    padding: 3px 12px;
    white-space: nowrap;
  }

  tbody tr {
    content-visibility: auto;
    contain-intrinsic-size: auto 22px;
  }

  .name {
    width: 100%;
    max-width: 0;
    overflow: hidden;
    font-family: var(--font-mono);
    text-overflow: ellipsis;
  }

  .dir .name {
    color: var(--accent);
  }

  .dir .num {
    color: var(--dim);
  }

  .num {
    text-align: right;
  }
</style>
