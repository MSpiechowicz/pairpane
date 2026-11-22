<script lang="ts">
  import { untrack } from 'svelte';

  import { displayName } from '$lib/panes/format';

  import ViewNotice from './ViewNotice.svelte';
  import { onNewHit, takeFocus, type ViewProps } from './viewProps.svelte';
  import ScrollTrack from './ScrollTrack.svelte';
  import type { ViewSession } from './session.svelte';
  import { LINE_HEIGHT, TextModel } from './textWindow.svelte';

  let { session, hit, focus = false }: ViewProps = $props();

  const model = new TextModel(untrack(() => session));
  let height = $state(0);

  const visible = $derived(model.lines.slice(model.first, model.first + model.rows + 1));

  $effect(() => {
    if (height > 0) {
      model.rows = Math.max(1, Math.floor(height / LINE_HEIGHT));
    }
  });

  // Starts the view (at the position the other view left) and starts it again in another encoding.
  $effect(() => {
    void session.encoding;
    untrack(() => void model.restart(session.position));
  });

  onNewHit(
    () => hit,
    (offset) => void model.restart(offset),
  );

  /** Whether the line at `index` of the buffer holds the match: it starts before it and the next one starts after. */
  function holds(index: number): boolean {
    if (hit === undefined) {
      return false;
    }

    const next = model.lines[index + 1]?.offset ?? (model.atEnd ? Number.POSITIVE_INFINITY : -1);

    return model.lines[index].offset <= hit.offset && hit.offset < next;
  }

  function onkeydown(event: KeyboardEvent) {
    if (model.key(event)) {
      event.preventDefault();
    }
  }
</script>

<div class="text">
  <div
    class="viewport"
    class:wrap={session.wrap}
    role="textbox"
    aria-readonly="true"
    aria-multiline="true"
    aria-label="Text of {displayName(session.name)}"
    data-wrap={session.wrap}
    tabindex="0"
    data-autofocus={focus ? '' : undefined}
    bind:clientHeight={height}
    {onkeydown}
    onwheel={(event) => model.wheel(event)}
    {@attach takeFocus(focus)}
  >
    {#each visible as line, at (line.offset)}
      <div class="line" class:hit={holds(model.first + at)} aria-current={holds(model.first + at) ? 'true' : undefined}>{line.text}</div>
    {/each}
  </div>
  <ViewNotice error={model.error} text={model.ready && model.lines.length === 0 ? 'The file is empty.' : undefined} />
  <ScrollTrack
    extent={session.size}
    position={model.thumb}
    visible={!(model.atStart && model.atEnd)}
    onSeek={(position) => void model.seek(position)}
  />
</div>

<style>
  .text {
    position: relative;
    flex: 1;
    min-height: 0;
  }

  .viewport {
    height: 100%;
    padding: 4px 22px 4px 12px;
    overflow: hidden;
    outline: none;
    font: 12.5px/18px var(--font-mono);
    tab-size: 4;
  }

  .line {
    min-height: 18px;
    white-space: pre;
  }

  .wrap .line {
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }

  .hit {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
</style>
