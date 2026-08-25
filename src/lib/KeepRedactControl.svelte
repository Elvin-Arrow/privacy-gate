<script lang="ts">
  // ui.md §2.3 keep/redact segmented control (NFR-U2): icon + label, never colour alone.
  import type { FieldDecisionKind } from './api'
  import { KEEP_LABEL, REDACT_LABEL } from './copy'

  let {
    value,
    onChange,
  }: {
    value: FieldDecisionKind | null
    onChange: (decision: FieldDecisionKind) => void
  } = $props()
</script>

<div class="segmented" role="group">
  <button
    type="button"
    class="segment"
    class:selected-keep={value === 'keep_visible'}
    aria-pressed={value === 'keep_visible'}
    onclick={() => onChange('keep_visible')}
  >
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M1 12s4-7 11-7 11 7 11 7-4 7-11 7-11-7-11-7Z"></path>
      <circle cx="12" cy="12" r="3"></circle>
    </svg>
    {KEEP_LABEL}
  </button>
  <button
    type="button"
    class="segment"
    class:selected-redact={value === 'redact'}
    aria-pressed={value === 'redact'}
    onclick={() => onChange('redact')}
  >
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M3 3l18 18"></path>
      <path d="M10.6 10.6a3 3 0 0 0 4.2 4.2"></path>
      <path d="M9.9 5.1A11 11 0 0 1 12 5c7 0 11 7 11 7a13 13 0 0 1-3.1 3.6M6.1 6.1C3.6 7.7 2 10 1 12c0 0 4 7 11 7 1.3 0 2.5-.2 3.6-.6"></path>
    </svg>
    {REDACT_LABEL}
  </button>
</div>

<style>
  .segmented {
    display: inline-flex;
    border: 1px solid var(--md-outline);
    border-radius: var(--md-radius-full);
    overflow: hidden;
    flex-shrink: 0;
  }

  .segment {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 30px;
    padding: 0 12px;
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.2px;
    background: var(--md-surface);
    color: var(--md-on-surface-variant);
    border: none;
    cursor: pointer;
    border-right: 1px solid var(--md-outline);
  }

  .segment:last-child {
    border-right: none;
  }

  .segment.selected-keep {
    background: var(--md-tertiary-container);
    color: var(--md-on-tertiary-container);
    font-weight: 700;
  }

  .segment.selected-redact {
    background: var(--md-error-container);
    color: var(--md-on-error-container);
    font-weight: 700;
  }
</style>
