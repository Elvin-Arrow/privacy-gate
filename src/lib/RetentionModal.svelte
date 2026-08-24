<script lang="ts">
  // ui.md §6 / decision 0007 — blocking first-import modal. Shown only when
  // `get_retention_default().confirmed === false`; Continue calls `set_retention_default`
  // and only *then* does the caller open the file picker or accept a queued drop (C-UI-4).
  // Cancel fires no command at all — `confirmed` stays false and the caller must not
  // proceed to import.
  //
  // Distinct from the compact per-import override control (ui.md §7.2's "Later imports"):
  // that one only ever appears once `confirmed === true` and maps to
  // `import_document.retention_override`, not `set_retention_default`. Kept as a separate
  // component (`RetentionOverrideControl.svelte`) rather than one component with a `mode`
  // prop — the two have different triggers (blocking vs. inline), different target
  // commands, and sharing a component would mean branching most of the template on `mode`
  // for little real reuse beyond the three label strings, which already live in
  // `copy.ts`/`RETENTION_POLICY_LABELS`.

  import { RETENTION_POLICY_LABELS } from './copy'
  import type { RetentionPolicy } from './api'
  import { RETENTION_MODAL_BODY, RETENTION_MODAL_TITLE } from './copy'

  const RETENTION_POLICIES: RetentionPolicy[] = ['discard', 'retain', 'never_retain']

  let {
    onContinue,
    onCancel,
  }: {
    onContinue: (policy: RetentionPolicy) => void
    onCancel: () => void
  } = $props()

  // §6: "Discard is pre-selected."
  let selected = $state<RetentionPolicy>('discard')
</script>

<div class="backdrop" role="presentation">
  <div class="modal" role="dialog" aria-modal="true" aria-labelledby="retention-modal-title">
    <h2 id="retention-modal-title">{RETENTION_MODAL_TITLE}</h2>
    <p class="body">{RETENTION_MODAL_BODY}</p>

    <fieldset>
      <legend class="visually-hidden">Default for original files</legend>
      {#each RETENTION_POLICIES as policy (policy)}
        <label class="radio-row">
          <input
            type="radio"
            name="retention-modal-policy"
            value={policy}
            checked={selected === policy}
            onchange={() => (selected = policy)}
          />
          {RETENTION_POLICY_LABELS[policy]}
        </label>
      {/each}
    </fieldset>

    <div class="actions">
      <button type="button" class="secondary" onclick={onCancel}>Cancel</button>
      <button type="button" class="primary" onclick={() => onContinue(selected)}>
        Continue
      </button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .modal {
    width: 420px;
    max-width: calc(100vw - 32px);
    padding: 24px;
    border-radius: var(--md-radius-xl);
    background: var(--md-surface-container-lowest);
    box-shadow: var(--md-elev-4);
  }

  h2 {
    margin: 0 0 12px;
    font-size: 16px;
    font-weight: 500;
    color: var(--md-on-surface);
  }

  .body {
    margin: 0 0 16px;
    font-size: 13px;
    line-height: 1.6;
    color: var(--md-on-surface-variant);
  }

  fieldset {
    border: none;
    padding: 0;
    margin: 0 0 20px;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }

  .radio-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13.5px;
    color: var(--md-on-surface);
    margin-bottom: 10px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 12px;
  }

  button {
    height: 40px;
    padding: 0 16px;
    border: none;
    border-radius: var(--md-radius-full);
    font-size: 13.5px;
    font-weight: 500;
    cursor: pointer;
  }

  .primary {
    background: var(--md-primary);
    color: var(--md-on-primary);
  }

  .secondary {
    background: var(--md-surface-container-lowest);
    color: var(--md-on-surface);
    border: 1px solid var(--md-outline-variant);
  }
</style>
