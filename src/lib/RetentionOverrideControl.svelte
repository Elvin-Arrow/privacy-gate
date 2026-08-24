<script lang="ts">
  // ui.md §7.2 "Later imports" compact control — inline, only rendered once
  // `confirmed === true` (the blocking `RetentionModal` owns everything before that).
  // Maps to `import_document.retention_override`: `null` (Use default), `"retain"`
  // (Keep original), `"discard"` (Discard original). When the global default is
  // `never_retain`, "Keep original" is disabled rather than hidden — clicking it explains
  // `retention_loosen_forbidden` (ui.md §7.2 / §15) instead of silently doing nothing.

  import type { EffectiveRetention } from './api'
  import { RETENTION_LOOSEN_FORBIDDEN_COPY, RETENTION_OVERRIDE_LABELS } from './copy'

  let {
    value,
    defaultIsNeverRetain,
    onChange,
  }: {
    value: EffectiveRetention | null
    defaultIsNeverRetain: boolean
    onChange: (next: EffectiveRetention | null) => void
  } = $props()

  let forbiddenNotice = $state(false)

  function selectRetain() {
    if (defaultIsNeverRetain) {
      forbiddenNotice = true
      return
    }
    forbiddenNotice = false
    onChange('retain')
  }

  function select(next: EffectiveRetention | null) {
    forbiddenNotice = false
    onChange(next)
  }
</script>

<div class="override-control">
  <span class="label">This import:</span>
  <div class="options" role="radiogroup" aria-label="Retention for this import">
    <button
      type="button"
      class="option"
      class:selected={value === null}
      onclick={() => select(null)}
    >
      {RETENTION_OVERRIDE_LABELS.default}
    </button>
    <button
      type="button"
      class="option"
      class:selected={value === 'retain'}
      disabled={defaultIsNeverRetain}
      onclick={selectRetain}
    >
      {RETENTION_OVERRIDE_LABELS.retain}
    </button>
    <button
      type="button"
      class="option"
      class:selected={value === 'discard'}
      onclick={() => select('discard')}
    >
      {RETENTION_OVERRIDE_LABELS.discard}
    </button>
  </div>
  {#if forbiddenNotice}
    <p class="notice" role="alert">{RETENTION_LOOSEN_FORBIDDEN_COPY}</p>
  {/if}
</div>

<style>
  .override-control {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }

  .label {
    color: var(--md-on-surface-variant);
  }

  .options {
    display: flex;
    gap: 6px;
  }

  .option {
    height: 30px;
    padding: 0 12px;
    border-radius: var(--md-radius-full);
    border: 1px solid var(--md-outline-variant);
    background: var(--md-surface-container-lowest);
    color: var(--md-on-surface);
    font-size: 12px;
    cursor: pointer;
  }

  .option.selected {
    border-color: var(--md-primary);
    color: var(--md-primary);
    font-weight: 500;
  }

  .option:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .notice {
    flex-basis: 100%;
    margin: 4px 0 0;
    font-size: 11.5px;
    color: var(--md-error);
  }
</style>
