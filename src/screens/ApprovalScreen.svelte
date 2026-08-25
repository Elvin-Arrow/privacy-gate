<script lang="ts">
  // ui.md §8 — two-pane approval (consent). Locatable spans on the left; keep/redact
  // list on the right (NFR-U2: not colour-only). First paint is the first page + first
  // 200 field rows (ui.md §14); the rest of the list paints after (§8 progressive).
  // C-UI-2: unapproved span text lives here until submit/abort/lock.

  import { onDestroy, onMount } from 'svelte'
  import AppShell from '../lib/AppShell.svelte'
  import KeepRedactControl from '../lib/KeepRedactControl.svelte'
  import { layoutPage } from '../lib/approvalLayout'
  import {
    abortApproval,
    isApiError,
    openApproval,
    setFieldDecisions,
    submitApproval,
    type ApprovalLifecycle,
    type ApprovalView,
    type FieldDecisionKind,
  } from '../lib/api'
  import {
    ALREADY_APPROVED_COPY,
    APPROVAL_BUSY_COPY,
    APPROVAL_CANCEL_LABEL,
    APPROVAL_DECIDED_COPY,
    APPROVAL_PENDING_COPY,
    APPROVAL_TITLE,
    APPROVE_AND_STORE_LABEL,
  } from '../lib/copy'

  const FIRST_PAINT_FIELD_CAP = 200

  let {
    docId,
    sourceFilename,
    onLock,
    onNavigateVault,
    onNavigateSettings,
    onNavigateAudit,
    onDone,
  }: {
    docId: string
    sourceFilename: string
    onLock: () => void
    onNavigateVault: () => void
    onNavigateSettings: () => void
    onNavigateAudit: () => void
    onDone: () => void
  } = $props()

  let view = $state<ApprovalView | null>(null)
  let lifecycle = $state<ApprovalLifecycle>('awaiting_decisions')
  let decisions = $state<Record<string, FieldDecisionKind>>({})
  let paintedFieldCount = $state(0)
  let selectedFieldId = $state<string | null>(null)
  let loadError = $state('')
  let actionError = $state('')
  let submitting = $state(false)
  let settled = $state(false)
  let cancelled = $state(false)
  let restPaintTimer: ReturnType<typeof setTimeout> | undefined

  const sessionId = $derived(view?.approval_session_id ?? null)
  const visibleFields = $derived(view ? view.fields.slice(0, paintedFieldCount) : [])
  const approveEnabled = $derived(lifecycle === 'decided' && !submitting && !loadError)

  onMount(() => {
    void load()
  })

  onDestroy(() => {
    cancelled = true
    if (restPaintTimer !== undefined) {
      clearTimeout(restPaintTimer)
    }
    if (sessionId && !settled) {
      void abortApproval(sessionId).catch(() => {
        // Lock (or a racing abort) may already have torn the session down.
      })
    }
  })

  async function load() {
    try {
      const opened = await openApproval(docId)
      if (cancelled || settled) {
        await abortApproval(opened.approval_session_id).catch(() => {})
        return
      }
      view = opened
      lifecycle = opened.lifecycle
      paintedFieldCount = Math.min(FIRST_PAINT_FIELD_CAP, opened.fields.length)
      if (opened.fields.length > FIRST_PAINT_FIELD_CAP) {
        restPaintTimer = setTimeout(() => {
          paintedFieldCount = opened.fields.length
        }, 0)
      }
    } catch (err) {
      if (cancelled || settled) return
      loadError = mapOpenError(err)
    }
  }

  function mapOpenError(err: unknown): string {
    if (isApiError(err)) {
      if (err.code === 'already_approved') return ALREADY_APPROVED_COPY
      if (err.code === 'approval_busy') return APPROVAL_BUSY_COPY
      return err.message
    }
    return 'Could not open this document for review.'
  }

  async function decide(fieldId: string, decision: FieldDecisionKind) {
    if (!sessionId || settled) return
    actionError = ''
    try {
      const out = await setFieldDecisions(sessionId, [{ field_id: fieldId, decision }])
      decisions = { ...decisions, [fieldId]: decision }
      lifecycle = out.lifecycle
    } catch (err) {
      actionError = isApiError(err) ? err.message : 'Could not save that decision.'
    }
  }

  async function handleSubmit() {
    if (!sessionId || !approveEnabled) return
    submitting = true
    actionError = ''
    try {
      await submitApproval(sessionId)
      settled = true
      onDone()
    } catch (err) {
      actionError = isApiError(err) ? err.message : 'Could not approve this document.'
    } finally {
      submitting = false
    }
  }

  async function handleCancel() {
    cancelled = true
    if (sessionId && !settled) {
      settled = true
      try {
        await abortApproval(sessionId)
      } catch {
        // Already torn down (lock) is not an error the user needs to see.
      }
    }
    onDone()
  }

  function selectField(id: string) {
    selectedFieldId = id
  }

  function handleOptionKeydown(event: KeyboardEvent, fieldId: string) {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
    event.preventDefault()
    const idx = visibleFields.findIndex((field) => field.id === fieldId)
    if (idx < 0) return
    const nextIdx =
      event.key === 'ArrowDown'
        ? Math.min(idx + 1, visibleFields.length - 1)
        : Math.max(idx - 1, 0)
    const next = visibleFields[nextIdx]
    if (!next) return
    selectedFieldId = next.id
    const el = document.getElementById(`field-option-${next.id}`)
    el?.focus()
  }

  function nestDepth(fieldId: string): number {
    if (!view) return 0
    let depth = 0
    let current = view.fields.find((field) => field.id === fieldId) ?? null
    const seen = new Set<string>()
    while (current?.parent_field_id && !seen.has(current.id)) {
      seen.add(current.id)
      depth += 1
      const parentId = current.parent_field_id
      current = view.fields.find((field) => field.id === parentId) ?? null
    }
    return depth
  }
</script>

<div class="screen">
  <AppShell active="vault" {onNavigateVault} {onNavigateAudit} {onNavigateSettings} {onLock} />

  <header class="topbar">
    <div class="titles">
      <h1>{APPROVAL_TITLE}</h1>
      {#if sourceFilename}
        <p class="filename">{sourceFilename}</p>
      {/if}
    </div>
    <div class="actions">
      {#if view && !loadError}
        <p class="status">
          {lifecycle === 'decided' ? APPROVAL_DECIDED_COPY : APPROVAL_PENDING_COPY}
        </p>
      {/if}
      <button type="button" class="btn-outlined" onclick={handleCancel}>
        {APPROVAL_CANCEL_LABEL}
      </button>
      {#if !loadError}
        <button type="button" class="btn-hero" disabled={!approveEnabled} onclick={handleSubmit}>
          {APPROVE_AND_STORE_LABEL}
        </button>
      {/if}
    </div>
  </header>

  {#if loadError}
    <p class="notice error" role="alert">{loadError}</p>
  {:else if view}
    <div class="panes">
      <section class="document-pane" aria-label="Document text">
        <p class="eyebrow">Document text</p>
        {#each view.pages as page (page.page_index)}
          {@const segments = layoutPage(page, view.fields, decisions)}
          <p class="page-text">{#each segments as segment, i (`${page.page_index}-${i}`)}{#if segment.fieldId}<button
                  type="button"
                  class="field-span"
                  class:kept={segment.kind === 'keep_visible'}
                  class:redacted={segment.kind === 'redact'}
                  class:undecided={segment.kind === 'undecided'}
                  class:nested={segment.nested}
                  data-testid={`field-span-${segment.fieldId}`}
                  data-selected={selectedFieldId === segment.fieldId ? 'true' : 'false'}
                  onclick={() => {
                    if (segment.fieldId) selectField(segment.fieldId)
                  }}
                >{segment.text}</button>{:else}{segment.text}{/if}{/each}</p>
        {/each}
      </section>

      <section class="fields-pane">
        <p class="eyebrow">Detected fields · {view.fields.length}</p>
        <div class="field-list">
          {#each visibleFields as field (field.id)}
            <div
              class="field-row"
              class:selected={selectedFieldId === field.id}
              style:padding-left="{20 + nestDepth(field.id) * 20}px"
            >
              <button
                id={`field-option-${field.id}`}
                type="button"
                class="field-select"
                aria-label={field.label}
                aria-pressed={selectedFieldId === field.id}
                onclick={() => selectField(field.id)}
                onkeydown={(event) => handleOptionKeydown(event, field.id)}
              >
                <span class="field-label">{field.label}</span>
                <span class="field-class">
                  {field.classification}{#if field.parent_field_id} · nested{/if}
                </span>
              </button>
              <KeepRedactControl
                value={decisions[field.id] ?? null}
                onChange={(decision) => void decide(field.id, decision)}
              />
            </div>
          {/each}
        </div>
      </section>
    </div>
  {/if}

  {#if actionError}
    <p class="notice error" role="alert">{actionError}</p>
  {/if}
</div>

<style>
  .screen {
    min-height: 100vh;
    background: var(--md-surface);
    font-family: var(--md-font);
    display: flex;
    flex-direction: column;
  }

  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 12px 24px;
    border-bottom: 1px solid var(--md-outline-variant);
  }

  .titles h1 {
    margin: 0;
    font-size: 20px;
    line-height: 26px;
    font-weight: 500;
  }

  .filename {
    margin: 2px 0 0;
    font-size: 12px;
    color: var(--md-on-surface-variant);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .status {
    margin: 0 4px 0 0;
    font-size: 12px;
    color: var(--md-on-surface-variant);
  }

  .btn-outlined {
    height: 40px;
    padding: 0 19px;
    border-radius: var(--md-radius-full);
    border: 1px solid var(--md-outline);
    background: transparent;
    color: var(--md-on-surface-variant);
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
  }

  .btn-hero {
    height: 40px;
    padding: 0 20px;
    border: none;
    border-radius: var(--md-radius-full);
    background: linear-gradient(135deg, var(--md-primary) 0%, var(--md-primary-dim) 100%);
    color: var(--md-on-primary);
    font-size: 14px;
    font-weight: 500;
    box-shadow: 0 6px 14px -6px color-mix(in oklch, var(--md-primary) 55%, transparent);
    cursor: pointer;
  }

  .btn-hero:disabled {
    background: color-mix(in oklch, var(--md-on-surface) 12%, transparent);
    color: color-mix(in oklch, var(--md-on-surface) 38%, transparent);
    box-shadow: none;
    cursor: not-allowed;
  }

  .panes {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  .document-pane {
    flex: 1;
    overflow: auto;
    padding: 32px 40px;
  }

  .fields-pane {
    width: 400px;
    min-width: 280px;
    border-left: 1px solid var(--md-outline-variant);
    display: flex;
    flex-direction: column;
    overflow: auto;
  }

  .eyebrow {
    margin: 0;
    padding: 20px 20px 8px;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.4px;
    text-transform: uppercase;
    color: var(--md-on-surface-variant);
  }

  .document-pane .eyebrow {
    padding: 0 0 16px;
  }

  .page-text {
    margin: 0 0 16px;
    max-width: 640px;
    font-size: 15px;
    line-height: 26px;
  }

  .field-span {
    font: inherit;
    padding: 1px 2px;
    border: none;
    border-radius: 2px;
    cursor: pointer;
  }

  .field-span.undecided {
    background: var(--md-surface-container-highest);
    border-bottom: 2px dashed var(--md-on-surface-variant);
  }

  .field-span.kept {
    background: color-mix(in oklch, var(--md-tertiary-container) 70%, transparent);
    border-bottom: 2px solid var(--md-on-tertiary-container);
  }

  .field-span.redacted {
    background: repeating-linear-gradient(
      -45deg,
      color-mix(in oklch, var(--md-error-container) 75%, transparent),
      color-mix(in oklch, var(--md-error-container) 75%, transparent) 4px,
      transparent 4px,
      transparent 8px
    );
    border-bottom: 2px solid var(--md-error);
  }

  .field-span[data-selected='true'] {
    outline: 2px solid var(--md-primary);
    outline-offset: 1px;
  }

  .field-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 20px;
    border-top: 1px solid var(--md-outline-variant);
  }

  .field-row.selected {
    background: color-mix(in oklch, var(--md-primary) 6%, transparent);
  }

  .field-select {
    min-width: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 0;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
    color: inherit;
    font: inherit;
  }

  .field-label {
    font-size: 14px;
    font-weight: 500;
    line-height: 20px;
  }

  .field-class {
    font-size: 12px;
    color: var(--md-on-surface-variant);
  }

  .notice {
    margin: 16px 24px;
    font-size: 13px;
  }

  .notice.error {
    color: var(--md-error);
  }
</style>
