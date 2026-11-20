<script lang="ts">
  let {
    label,
    path,
    active,
    onActivate,
  }: {
    /** Accessible name of the pane ("Left pane"). */
    label: string;
    /** The active tab's folder, one entry per breadcrumb segment, starting at the drive. */
    path: readonly string[];
    active: boolean;
    onActivate: () => void;
  } = $props();
</script>

<!-- Clicking anywhere in a pane makes it the active one; Tab does the same from the keyboard (App). -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<section class="pane" data-pane class:active aria-label={label} aria-current={active ? 'true' : undefined} onclick={onActivate}>
  <div class="phead">
    <button class="drive" type="button" aria-label="Drives">
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 7.5 8 3l5.5 4.5V13h-4v-3h-3v3h-4z" /></svg>
      <span class="caret">▾</span>
    </button>
    <!-- Tab roles arrive with real tabs (#9); until then the active tab is just the path. -->
    <div class="tabs">
      <div class="tab on">
        <nav class="crumbs" aria-label="Path">
          {#each path as segment, index (index)}
            {#if index > 0}<i>/</i>{/if}
            <span class:last={index === path.length - 1}>{segment}</span>
          {/each}
        </nav>
      </div>
      <button class="tab add" type="button" aria-label="New tab">+</button>
    </div>
  </div>

  <div class="head">
    <span></span><span>Name</span><span>Ext</span><span class="r">Size</span><span>Modified</span>
  </div>
  <div class="rows"></div>
  <div class="foot"></div>
</section>

<style>
  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--panel);
    transition: border-color 0.12s;
  }

  .pane.active {
    border-color: color-mix(in srgb, var(--accent) 60%, var(--line));
  }

  /* One header row per pane: one height, one font size. */
  .phead {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 40px;
    padding: 0 6px;
    border-bottom: 1px solid var(--line);
    background: var(--panel-2);
    font-size: 12.5px;
  }

  .drive {
    flex: none;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 6px 0 8px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--panel);
    color: var(--text);
  }

  .drive:hover,
  .active .drive {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--line));
  }

  .drive svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linejoin: round;
  }

  .caret {
    font-size: 9px;
    color: var(--dim);
  }

  .tabs {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    overflow: hidden;
  }

  .tab {
    flex: none;
    height: 28px;
    display: flex;
    align-items: center;
    padding: 0 9px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: none;
    color: var(--dim);
  }

  .tab.on {
    flex: 1 1 auto;
    min-width: 0;
    padding: 0 2px;
    border-color: var(--line);
    background: var(--panel);
    color: var(--text);
  }

  .crumbs {
    display: flex;
    align-items: center;
    gap: 1px;
    overflow: hidden;
    white-space: nowrap;
  }

  .crumbs span {
    padding: 2px 5px;
    color: var(--dim);
  }

  .crumbs span.last {
    color: var(--text);
    font-weight: 600;
  }

  .crumbs i {
    font-style: normal;
    color: var(--line);
  }

  .head {
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) 48px 74px 118px;
    align-items: center;
    gap: 8px;
    height: 25px;
    padding: 0 10px;
    border-bottom: 1px solid var(--line);
    font-size: 11px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--dim);
    user-select: none;
  }

  .head .r {
    text-align: right;
  }

  .rows {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }

  .foot {
    height: 28px;
    border-top: 1px solid var(--line);
  }

  @media (max-width: 760px) {
    .head {
      grid-template-columns: 20px minmax(0, 1fr) 74px;
    }

    .head span:nth-child(3),
    .head span:nth-child(5) {
      display: none;
    }
  }
</style>
