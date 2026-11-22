<script lang="ts">
  import { loadMerge, type MergeSurface } from '$lib/editor/surface';

  import DiffFrame from './DiffFrame.svelte';
  import DiffNote from './DiffNote.svelte';
  import DiffSideHeader from './DiffSideHeader.svelte';
  import DiffToolbar from './DiffToolbar.svelte';
  import type { FileDiff, TextPair } from './fileDiff.svelte';

  let { diff, pair }: { diff: FileDiff; pair: TextPair } = $props();

  let merge = $state.raw<MergeSurface | undefined>();
  let changes = $state(0);
  let collapsed = $state(false);

  const summary = $derived.by(() => {
    if (merge === undefined) {
      return 'Comparing…';
    }

    if (changes === 0) {
      return 'No differences';
    }

    return changes === 1 ? '1 change' : `${changes} changes`;
  });

  /** Puts the two editing widgets (loaded on first use) in `node`; each session hears of the changes of its side. */
  function mount(node: HTMLElement) {
    let surface: MergeSurface | undefined;
    let gone = false;

    loadMerge()
      .then(({ createMergeSurface }) => {
        if (gone) {
          return;
        }

        surface = createMergeSurface({
          parent: node,
          left: { name: pair.left.name, text: pair.left.text(), readOnly: pair.left.readOnly },
          right: { name: pair.right.name, text: pair.right.text(), readOnly: pair.right.readOnly },
          onSave: (side) => void diff.save(side),
          onChunks: (count) => (changes = count),
        });
        pair.left.attach(surface.left);
        pair.right.attach(surface.right);
        surface.setCollapsed(collapsed);
        merge = surface;
        surface.focus('left');
      })
      .catch(() => (pair.left.failure = 'The comparison could not be loaded.'));

    return () => {
      gone = true;
      merge = undefined;
      if (surface !== undefined) {
        pair.left.detach();
        pair.right.detach();
        surface.destroy();
      }
    };
  }

  function toggleCollapsed() {
    collapsed = !collapsed;
    merge?.setCollapsed(collapsed);
  }

  /** Ctrl+S saves the side that has (or last had) the focus; in the text it is the surface's own, and the page hears of it once. F7 and Shift+F7 move between the changes. */
  function onkeydown(event: KeyboardEvent) {
    if (event.isComposing || event.altKey || event.metaKey || event.defaultPrevented) {
      return;
    }

    if (event.key.toLowerCase() === 's' && event.ctrlKey && !event.shiftKey) {
      event.preventDefault();
      if (diff.question === undefined) {
        void diff.save(merge?.focusedSide() ?? 'left');
      }
    } else if (event.key === 'F7' && !event.ctrlKey) {
      event.preventDefault();
      if (event.shiftKey) {
        merge?.previous();
      } else {
        merge?.next();
      }
    }
  }
</script>

<svelte:window {onkeydown} />

<DiffFrame>
  <DiffToolbar {summary} disabled={changes === 0} onPrevious={() => merge?.previous()} onNext={() => merge?.next()} onClose={() => void diff.requestClose()}>
    <button class="btn" type="button" aria-pressed={collapsed} onclick={toggleCollapsed}>Collapse unchanged</button>
  </DiffToolbar>

  {#each diff.notes as note (note)}
    <DiffNote text={note} />
  {/each}

  <div class="heads">
    <DiffSideHeader session={pair.left} label="Left file" onSave={() => void diff.save('left')} />
    <DiffSideHeader session={pair.right} label="Right file" onSave={() => void diff.save('right')} />
  </div>

  <div class="host" {@attach mount}></div>
</DiffFrame>

<style>
  .heads {
    display: flex;
  }

  .heads > :global(* + *) {
    border-left: 1px solid var(--line);
  }

  .host {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .host :global(.cm-mergeView) {
    height: 100%;
    overflow: auto;
  }

  button[aria-pressed='true'] {
    border-color: color-mix(in srgb, var(--accent) 60%, var(--line));
    background: color-mix(in srgb, var(--accent) 16%, var(--panel-2));
  }
</style>
