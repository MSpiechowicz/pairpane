<script lang="ts">
  import type { FindHit } from '$lib/ipc/bindings/FindHit';
  import type { Tri } from '$lib/ipc/bindings/Tri';
  import { enterSubmits } from '$lib/ops/submission.svelte';
  import { formatSize } from '$lib/panes/format';
  import { isWindowsPlatform } from '$lib/panes/keys';
  import Modal from '$lib/shell/Modal.svelte';
  import { ENCODINGS } from '$lib/viewer/encodings';

  import FindResults from './FindResults.svelte';
  import type { Finder, FindSummary, ResultsTarget } from './finder.svelte';
  import { blocked, SIZE_UNITS, type SizeUnit, toCriteria, toForm } from './query';
  import SavedBar from './SavedBar.svelte';
  import type { SavedSearches } from './saved.svelte';
  import '../ops/fields.css';

  let {
    finder,
    saved,
    results,
    onGoTo,
  }: {
    finder: Finder;
    saved: SavedSearches;
    /** Where "Feed to panel" lists the results: the active pane. */
    results: ResultsTarget;
    /** Opens the folder of a result in the active pane. */
    onGoTo: (hit: FindHit) => void;
  } = $props();

  const windows = isWindowsPlatform();
  const form = $derived(finder.form);

  const SIZES = [
    { label: 'At least', size: 'minSize', unit: 'minUnit' },
    { label: 'At most', size: 'maxSize', unit: 'maxUnit' },
  ] as const;

  /** The execute bit is not a thing on Windows. */
  const ATTRIBUTES = [
    { label: 'Hidden', key: 'hidden' },
    { label: 'Read-only', key: 'readonly' },
    { label: 'Executable', key: 'executable' },
  ] as const;
  const attributes = ATTRIBUTES.filter(({ key }) => !windows || key !== 'executable');

  const lock = $derived(blocked(form));
  let selected = $state(-1);

  const hit = $derived.by(() => {
    const row = finder.rows[selected];
    return row?.kind === 'hit' ? row.hit : undefined;
  });

  function criteria() {
    return toCriteria(form, windows);
  }

  function start() {
    const asked = criteria();
    if (typeof asked === 'string') {
      finder.refuse(asked);
    } else {
      void finder.start(asked);
    }
  }

  function startOrStop() {
    if (finder.running) {
      finder.stop();
    } else {
      start();
    }
  }

  function goTo(found: FindHit) {
    onGoTo(found);
    finder.close();
  }

  async function feed() {
    if (await finder.feed(results)) {
      finder.close();
    }
  }

  const count = (n: number, noun: string) => `${n.toLocaleString('en-US')} ${noun}${n === 1 ? '' : 's'}`;

  /** The line under the Start button once the search is over. */
  function summary({ found, skipped, truncated, cancelled }: FindSummary): string {
    const parts = [`${cancelled ? 'Stopped: ' : ''}${count(found, 'result')}`];
    if (skipped > 0) {
      parts.push(`${skipped.toLocaleString('en-US')} skipped (unreadable or too big)`);
    }

    if (truncated) {
      parts.push('the search stopped at its limit: narrow it to see more');
    }

    return parts.join(' · ');
  }

  /** The line under the Start button while the search runs. */
  function progress(): string {
    const { progress: seen } = finder;
    if (seen === undefined) {
      return 'Looking…';
    }

    const bytes = seen.bytes > 0 ? ` · ${formatSize(seen.bytes)} ${form.duplicates ? 'hashed' : 'read'}` : '';
    return `Looking… ${count(seen.folders, 'folder')} · ${count(seen.files, 'file')}${bytes}`;
  }
</script>

<Modal title="Find files" variant="wide" onClose={() => finder.close()}>
  <div class="find" {@attach enterSubmits(startOrStop)}>
    <p class="root">Look in <span class="path">{finder.root}</span></p>

    <SavedBar {saved} {criteria} onLoad={(loaded) => Object.assign(form, toForm(loaded))} />

    <div class="form">
      <fieldset>
        <legend>Name</legend>
        <label class="labelled">
          <span>Names</span>
          <input
            class="field"
            type="text"
            spellcheck="false"
            autocomplete="off"
            placeholder="*.txt; *.md (blank: every name)"
            data-autofocus
            bind:value={form.names}
          />
        </label>
        <label class="check"><input type="checkbox" bind:checked={form.nameRegex} />Regular expression</label>
        <label class="check"><input type="checkbox" bind:checked={form.namePath} />On the path</label>
        <label class="check"><input type="checkbox" bind:checked={form.nameCase} />Match case</label>
      </fieldset>

      <fieldset>
        <legend>Content</legend>
        <label class="labelled">
          <span>Text</span>
          <input class="field" type="text" spellcheck="false" autocomplete="off" disabled={lock.text} bind:value={form.text} />
        </label>
        <label class="labelled">
          <span>Encoding</span>
          <select
            class="field"
            disabled={lock.text || form.text === ''}
            value={form.encoding}
            onchange={(event) => (form.encoding = event.currentTarget.value)}
          >
            <option value="">Automatic (UTF-8)</option>
            {#each ENCODINGS as encoding (encoding)}
              <option value={encoding}>{encoding}</option>
            {/each}
          </select>
        </label>
        <label class="check"><input type="checkbox" disabled={lock.text} bind:checked={form.textRegex} />Regular expression</label>
        <label class="check"><input type="checkbox" disabled={lock.text} bind:checked={form.textCase} />Match case</label>
        <label class="check"><input type="checkbox" disabled={lock.text} bind:checked={form.wholeWord} />Whole word</label>
        <label class="check"><input type="checkbox" disabled={lock.text} bind:checked={form.anySize} />Files of any size</label>
      </fieldset>

      <fieldset>
        <legend>Size</legend>
        {#each SIZES as { label, size, unit } (size)}
          <label class="labelled">
            <span>{label}</span>
            <input class="field number" type="text" inputmode="decimal" autocomplete="off" bind:value={form[size]} />
          </label>
          <select
            class="field"
            aria-label="{label} unit"
            value={form[unit]}
            onchange={(event) => (form[unit] = event.currentTarget.value as SizeUnit)}
          >
            {#each SIZE_UNITS as { label: name } (name)}
              <option value={name}>{name}</option>
            {/each}
          </select>
        {/each}
      </fieldset>

      <fieldset>
        <legend>Modified</legend>
        <label class="labelled">
          <span>From</span>
          <input class="field" type="date" bind:value={form.from} />
        </label>
        <label class="labelled">
          <span>To</span>
          <input class="field" type="date" bind:value={form.to} />
        </label>
      </fieldset>

      <fieldset>
        <legend>Attributes</legend>
        {#each attributes as { label, key } (key)}
          <label class="labelled">
            <span>{label}</span>
            <select class="field" value={form[key]} onchange={(event) => (form[key] = event.currentTarget.value as Tri)}>
              <option value="any">Any</option>
              <option value="yes" disabled={key === 'hidden' && lock.hiddenYes}>Yes</option>
              <option value="no">No</option>
            </select>
          </label>
        {/each}
      </fieldset>

      <fieldset>
        <legend>Options</legend>
        <label class="check"><input type="checkbox" disabled={lock.skipHidden} bind:checked={form.skipHidden} />Skip hidden</label>
        <label class="check"><input type="checkbox" bind:checked={form.skipIgnored} />Skip ignored (.gitignore)</label>
        <label class="check"><input type="checkbox" bind:checked={form.oneFilesystem} />One filesystem</label>
        <label class="check"><input type="checkbox" disabled={lock.archives} bind:checked={form.archives} />Inside archives</label>
        <label class="check"><input type="checkbox" disabled={lock.duplicates} bind:checked={form.duplicates} />Duplicates</label>
      </fieldset>
    </div>
    {#if form.archives}
      <p class="hint">Archives are searched by name only: zip, tar, 7z.</p>
    {/if}

    <div class="bar">
      <button class="btn btn-primary" type="button" disabled={finder.stopping} onclick={startOrStop}>
        {finder.running ? 'Stop' : 'Start'}
      </button>
      {#if finder.failure}
        <p class="failure" role="alert">{finder.failure}</p>
      {:else if finder.running}
        <p class="status">{progress()}</p>
      {:else if finder.summary}
        <p class="status" role="status">{summary(finder.summary)}</p>
      {/if}
    </div>

    <FindResults {finder} bind:selected onOpen={goTo} />
  </div>

  <div class="mt-3 flex items-center justify-end gap-2">
    {#if finder.inside > 0 && finder.feedable}
      <span class="note">Entries inside archives are not listed in a panel.</span>
    {/if}
    <button class="btn" type="button" onclick={() => finder.close()}>Close</button>
    <button class="btn" type="button" disabled={!finder.feedable} onclick={feed}>Feed to panel</button>
    <button class="btn btn-primary" type="button" disabled={hit === undefined} onclick={() => hit && goTo(hit)}>Go to</button>
  </div>
</Modal>

<style>
  .find {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .root {
    margin: 0;
    color: var(--dim);
  }

  .path {
    color: var(--text);
    font-family: var(--font-mono);
    unicode-bidi: isolate; /* the folder may hold right-to-left names: it is set apart from the text around it */
  }

  .form {
    display: grid;
    gap: 8px;
    max-height: min(300px, 34vh);
    padding-right: 4px;
    overflow-y: auto;
  }

  fieldset {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 14px;
    min-width: 0;
    margin: 0;
    padding: 6px 10px 8px;
    border: 1px solid var(--line);
    border-radius: 7px;
  }

  legend {
    padding: 0 4px;
    color: var(--dim);
    font-size: 11.5px;
  }

  .labelled {
    display: flex;
    flex: 1 1 200px;
    align-items: center;
    gap: 8px;
    min-width: 0;
    color: var(--dim);
  }

  .labelled .field {
    flex: 1;
    min-width: 0;
  }

  .labelled .number {
    max-width: 110px;
  }

  .hint {
    margin: 0;
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .bar p {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .status {
    color: var(--dim);
  }

  .failure {
    color: var(--danger);
  }

  .note {
    margin-right: auto;
    color: var(--dim);
  }
</style>
