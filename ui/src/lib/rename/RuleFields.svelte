<script lang="ts">
  import { tick } from 'svelte';

  import type { CaseChange } from '$lib/ipc/bindings/CaseChange';
  import type { CasePart } from '$lib/ipc/bindings/CasePart';
  import type { RuleField } from '$lib/ipc/bindings/RuleField';
  import { displayName } from '$lib/panes/format';

  import type { Renamer } from './renamer.svelte';
  import { CASE_PARTS, CASES, COUNTER_FIELDS, insertToken, PLACEHOLDERS, wholeNumber } from './rule';
  import '../ops/fields.css';

  let { renamer }: { renamer: Renamer } = $props();

  type Template = 'name' | 'ext';

  const inputs: Partial<Record<Template, HTMLInputElement>> = {};
  /** The template the placeholder buttons write into: the one that had the caret last. */
  let target: Template = 'name';

  /** The fault of the rule when it is in one of `fields`. */
  function fault(fields: RuleField[]): string | undefined {
    const { error } = renamer;
    return error !== undefined && fields.includes(error.field) ? error.message : undefined;
  }

  const invalid = (field: RuleField) => (fault([field]) === undefined ? undefined : true);

  function insert(token: string) {
    const input = inputs[target];
    if (input === undefined) {
      return;
    }

    const start = input.selectionStart ?? input.value.length;
    const end = input.selectionEnd ?? start;
    const next = insertToken(renamer.rule[target], start, end, token);
    renamer.rule[target] = next.value;

    void tick().then(() => {
      input.focus();
      input.setSelectionRange(next.caret, next.caret);
    });
  }

  function count(key: (typeof COUNTER_FIELDS)[number]['key'], max: number, text: string) {
    const value = wholeNumber(text, max);
    if (value !== undefined) {
      renamer.rule.counter[key] = value;
    }
  }
</script>

{#snippet message(fields: RuleField[])}
  {@const text = fault(fields)}
  {#if text !== undefined}
    <p class="failure" role="alert">{displayName(text)}</p>
  {/if}
{/snippet}

<div class="rule">
  <div class="templates">
    <label class="labelled">
      <span>Name</span>
      <input
        class="field"
        type="text"
        spellcheck="false"
        autocomplete="off"
        aria-invalid={invalid('name')}
        data-autofocus
        bind:this={inputs.name}
        onfocus={() => (target = 'name')}
        bind:value={renamer.rule.name}
      />
    </label>
    <label class="labelled ext">
      <span>Extension</span>
      <input
        class="field"
        type="text"
        spellcheck="false"
        autocomplete="off"
        aria-invalid={invalid('ext')}
        bind:this={inputs.ext}
        onfocus={() => (target = 'ext')}
        bind:value={renamer.rule.ext}
      />
    </label>
  </div>
  {@render message(['name', 'ext'])}
  <div class="tokens" role="group" aria-label="Placeholders">
    {#each PLACEHOLDERS as { token, label } (token)}
      <button class="btn token" type="button" title={label} onclick={() => insert(token)}>{token}</button>
    {/each}
  </div>

  <div class="counter" role="group" aria-label="Counter">
    {#each COUNTER_FIELDS as { key, label, max } (key)}
      <label class="labelled">
        <span>{label}</span>
        <input
          class="field number"
          type="number"
          min="0"
          {max}
          autocomplete="off"
          aria-invalid={invalid('counter')}
          value={renamer.rule.counter[key]}
          oninput={(event) => count(key, max, event.currentTarget.value)}
        />
      </label>
    {/each}
  </div>
  {@render message(['counter'])}

  <div class="templates">
    <label class="labelled">
      <span>Search</span>
      <input
        class="field"
        type="text"
        spellcheck="false"
        autocomplete="off"
        aria-invalid={invalid('search')}
        bind:value={renamer.rule.search}
      />
    </label>
    <label class="labelled">
      <span>Replace with</span>
      <input
        class="field"
        type="text"
        spellcheck="false"
        autocomplete="off"
        aria-invalid={invalid('replace')}
        bind:value={renamer.rule.replace}
      />
    </label>
  </div>
  {@render message(['search', 'replace'])}
  <div class="options">
    <label class="check"><input type="checkbox" bind:checked={renamer.rule.regex} />Regular expression</label>
    <label class="check"><input type="checkbox" bind:checked={renamer.rule.matchCase} />Match case</label>
    <label class="check"><input type="checkbox" bind:checked={renamer.rule.firstOnly} />First match only</label>
  </div>

  <div class="templates">
    <label class="labelled">
      <span>Letters</span>
      <select class="field" value={renamer.rule.case} onchange={(event) => (renamer.rule.case = event.currentTarget.value as CaseChange)}>
        {#each CASES as { value, label } (value)}
          <option {value}>{label}</option>
        {/each}
      </select>
    </label>
    <label class="labelled">
      <span>Applies to</span>
      <select
        class="field"
        disabled={renamer.rule.case === 'keep'}
        value={renamer.rule.casePart}
        onchange={(event) => (renamer.rule.casePart = event.currentTarget.value as CasePart)}
      >
        {#each CASE_PARTS as { value, label } (value)}
          <option {value}>{label}</option>
        {/each}
      </select>
    </label>
  </div>
</div>

<style>
  .rule {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .templates,
  .counter,
  .options,
  .tokens {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 14px;
  }

  .tokens {
    gap: 4px;
  }

  .token {
    padding-inline: 8px;
    font-family: var(--font-mono);
  }

  .counter .labelled {
    flex: 0 1 auto;
  }

  .labelled .number {
    flex: none;
    width: 110px;
  }

  .field[aria-invalid='true'] {
    border-color: var(--danger);
  }

  .failure {
    margin: 0;
    color: var(--danger);
  }
</style>
