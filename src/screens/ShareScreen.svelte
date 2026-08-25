<script lang="ts">
  // ui.md §10 — share preview + save-dialog chrome (OQ-4 / C-ARCH-2). W36 adds Ask Cloud
  // AI confirm (payload read-only, §15 copy before commit) and save_variant from the
  // share override set. Always preview_share before commit_share (C-UI-3). Dialog cancel
  // never commits. Write-fail retries the save without a second commit.

  import { onDestroy, onMount } from 'svelte'
  import { save } from '@tauri-apps/plugin-dialog'
  import { writeFile } from '@tauri-apps/plugin-fs'
  import { documentDir, join } from '@tauri-apps/api/path'
  import AppShell from '../lib/AppShell.svelte'
  import KeepRedactControl from '../lib/KeepRedactControl.svelte'
  import {
    commitShare,
    isApiError,
    listVariants,
    previewShare,
    saveVariant,
    type FieldDecisionDto,
    type FieldDecisionKind,
    type ShareKind,
    type SharePreview,
    type ShareRequestDto,
    type VariantSummary,
  } from '../lib/api'
  import {
    AI_CONFIRM_COPY,
    AI_PREVIEW_LABEL,
    APPROVAL_CANCEL_LABEL,
    ASK_CLOUD_AI_LABEL,
    CLOUD_AI_NOT_CONFIGURED_COPY,
    EPHEMERAL_OVERRIDE_COPY,
    EXPORT_PDF_LABEL,
    PREVIEW_EXPIRED_COPY,
    RETRY_SAVE_LABEL,
    SAVE_REDACTED_PDF_LABEL,
    SAVE_VARIANT_LABEL,
    OPEN_SETTINGS_LABEL,
    SEND_TO_AI_LABEL,
    SHARE_AI_FAILED_COPY,
    SHARE_TITLE,
    SHARE_WRITE_FAILED_COPY,
    VARIANT_NAME_CONFLICT_COPY,
  } from '../lib/copy'

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

  let preview = $state<SharePreview | null>(null)
  let blobUrl = $state<string | null>(null)
  let loadError = $state('')
  let actionError = $state('')
  let saving = $state(false)
  let committedBytes = $state<number[] | null>(null)
  let auditEventId = $state<number | null>(null)
  let writeFailed = $state(false)
  let savedFilename = $state('')
  let savedPath = $state('')

  let kind = $state<ShareKind>('export_to_person')
  let aiInstruction = $state('')
  let aiOutput = $state('')
  let overrides = $state<FieldDecisionDto[]>([])
  let appliedVariantId = $state('')
  let variants = $state<VariantSummary[]>([])
  let variantName = $state('')
  let variantStatus = $state('')
  let canonicalKeep = $state<Set<string>>(new Set())
  let canonicalCaptured = $state(false)
  // Bumped on every loadPreview / switchKind so an in-flight person-export preview
  // cannot land after the user has switched to Ask Cloud AI (or vice versa).
  let previewSeq = 0

  function revokeBlob() {
    if (blobUrl) {
      URL.revokeObjectURL(blobUrl)
      blobUrl = null
    }
  }

  function bytesToUint8(bytes: number[]): Uint8Array {
    return Uint8Array.from(bytes)
  }

  function pdfBlob(bytes: number[]): Blob {
    const buf = new ArrayBuffer(bytes.length)
    new Uint8Array(buf).set(bytes)
    return new Blob([buf], { type: 'application/pdf' })
  }

  function basename(path: string): string {
    const parts = path.split(/[/\\]/)
    return parts[parts.length - 1] ?? path
  }

  function buildRequest(): ShareRequestDto {
    const per_doc_overrides: Record<string, FieldDecisionDto[]> = {}
    if (overrides.length > 0) per_doc_overrides[docId] = overrides
    const applied_variant_ids: Record<string, string> = {}
    if (appliedVariantId) applied_variant_ids[docId] = appliedVariantId
    return {
      kind,
      doc_ids: [docId],
      per_doc_overrides,
      applied_variant_ids,
      recipient_note: null,
      ai_instruction: kind === 'share_to_ai' ? aiInstruction.trim() : null,
    }
  }

  function applyPreview(next: SharePreview) {
    revokeBlob()
    preview = next
    if (next.pdf_bytes && next.pdf_bytes.length > 0) {
      blobUrl = URL.createObjectURL(pdfBlob(next.pdf_bytes))
    }
    if (!canonicalCaptured) {
      canonicalKeep = new Set(next.manifest.flatMap((e) => e.visible_field_ids))
      canonicalCaptured = true
    }
  }

  async function loadPreview() {
    const seq = ++previewSeq
    if (kind === 'share_to_ai' && aiInstruction.trim().length === 0) {
      loadError = 'Enter an instruction before previewing.'
      return
    }
    loadError = ''
    writeFailed = false
    committedBytes = null
    auditEventId = null
    savedFilename = ''
    aiOutput = ''
    try {
      const out = await previewShare(buildRequest())
      if (seq !== previewSeq) return
      applyPreview(out)
    } catch (err) {
      if (seq !== previewSeq) return
      preview = null
      revokeBlob()
      if (isApiError(err) && err.code === 'cloud_ai_not_configured') {
        loadError = CLOUD_AI_NOT_CONFIGURED_COPY
        return
      }
      loadError = isApiError(err) ? err.message : 'Could not build a share preview.'
    }
  }

  async function defaultSavePath(suggested: string): Promise<string> {
    try {
      const dir = await documentDir()
      return await join(dir, suggested)
    } catch {
      return suggested
    }
  }

  async function persistBytes(path: string, bytes: number[]) {
    await writeFile(path, bytesToUint8(bytes))
    savedFilename = basename(path)
    savedPath = path
    writeFailed = false
    revokeBlob()
  }

  async function handleSave() {
    if (!preview || saving || kind !== 'export_to_person') return
    actionError = ''
    saving = true
    try {
      const suggested = preview.suggested_filename ?? 'redacted.pdf'
      const path = await save({
        title: SAVE_REDACTED_PDF_LABEL,
        defaultPath: await defaultSavePath(suggested),
        filters: [{ name: 'PDF', extensions: ['pdf'] }],
      })
      if (!path) {
        return
      }

      let bytes = committedBytes
      if (!bytes) {
        try {
          const out = await commitShare(preview.preview_token)
          bytes = out.pdf_bytes
          committedBytes = bytes
          auditEventId = out.audit_event_id
        } catch (err) {
          if (isApiError(err) && err.code === 'preview_expired') {
            actionError = PREVIEW_EXPIRED_COPY
            await loadPreview()
            return
          }
          actionError = isApiError(err) ? err.message : 'Could not complete the export.'
          return
        }
      }
      if (!bytes) {
        actionError = 'Could not complete the export.'
        return
      }

      try {
        await persistBytes(path, bytes)
      } catch {
        writeFailed = true
      }
    } finally {
      saving = false
    }
  }

  async function handleRetrySave() {
    if (!committedBytes || saving) return
    actionError = ''
    saving = true
    try {
      const suggested = preview?.suggested_filename ?? 'redacted.pdf'
      const path = await save({
        title: SAVE_REDACTED_PDF_LABEL,
        defaultPath: await defaultSavePath(suggested),
        filters: [{ name: 'PDF', extensions: ['pdf'] }],
      })
      if (!path) return
      try {
        await persistBytes(path, committedBytes)
      } catch {
        writeFailed = true
      }
    } finally {
      saving = false
    }
  }

  async function handleSendAi() {
    if (!preview || preview.kind !== 'share_to_ai' || saving) return
    actionError = ''
    saving = true
    try {
      const out = await commitShare(preview.preview_token)
      aiOutput = out.output_text ?? ''
      auditEventId = out.audit_event_id
    } catch (err) {
      if (isApiError(err) && err.code === 'preview_expired') {
        actionError = PREVIEW_EXPIRED_COPY
        await loadPreview()
        return
      }
      if (isApiError(err) && (err.code === 'cloud_ai_network' || err.code === 'cloud_ai_refused')) {
        actionError = `${err.message} ${SHARE_AI_FAILED_COPY}`
        return
      }
      actionError = isApiError(err) ? err.message : 'Could not send to Cloud AI.'
    } finally {
      saving = false
    }
  }

  async function handleSaveVariant() {
    const name = variantName.trim()
    if (name.length < 1 || name.length > 80) return
    variantStatus = ''
    try {
      await saveVariant({ doc_id: docId, name, overrides })
      variantName = ''
      variantStatus = 'Variant saved.'
      const listed = await listVariants(docId)
      variants = listed.variants
    } catch (err) {
      variantStatus =
        isApiError(err) && err.code === 'variant_name_conflict'
          ? VARIANT_NAME_CONFLICT_COPY
          : isApiError(err)
            ? err.message
            : 'Could not save the variant.'
    }
  }

  function decisionFor(fieldId: string, listedAsVisible: boolean): FieldDecisionKind {
    const override = overrides.find((o) => o.field_id === fieldId)
    if (override) return override.decision
    return listedAsVisible ? 'keep_visible' : 'redact'
  }

  function handleFieldDecision(fieldId: string, decision: FieldDecisionKind) {
    const canonical: FieldDecisionKind = canonicalKeep.has(fieldId) ? 'keep_visible' : 'redact'
    const rest = overrides.filter((o) => o.field_id !== fieldId)
    overrides = decision === canonical ? rest : [...rest, { field_id: fieldId, decision }]
    void loadPreview()
  }

  function switchKind(next: ShareKind) {
    if (kind === next) return
    previewSeq += 1
    kind = next
    preview = null
    revokeBlob()
    loadError = ''
    actionError = ''
    aiOutput = ''
    savedFilename = ''
    if (next === 'export_to_person') {
      void loadPreview()
    }
  }

  onMount(() => {
    listVariants(docId)
      .then((out) => {
        variants = out.variants
      })
      .catch(() => {
        variants = []
      })
    void loadPreview()
  })

  onDestroy(() => {
    revokeBlob()
  })
</script>

<div class="screen">
  <AppShell active="vault" {onNavigateVault} {onNavigateAudit} {onNavigateSettings} {onLock} />

  <header class="topbar">
    <div class="titles">
      <h1>{SHARE_TITLE}</h1>
      {#if sourceFilename}
        <p class="filename">{sourceFilename}</p>
      {/if}
    </div>
    <div class="actions">
      <button type="button" class="btn-outlined" onclick={onDone}>
        {APPROVAL_CANCEL_LABEL}
      </button>
      {#if writeFailed}
        <button type="button" class="btn-hero" disabled={saving} onclick={handleRetrySave}>
          {RETRY_SAVE_LABEL}
        </button>
      {:else if kind === 'share_to_ai' && preview?.kind === 'share_to_ai' && !aiOutput}
        <button type="button" class="btn-hero" disabled={saving} onclick={handleSendAi}>
          {SEND_TO_AI_LABEL}
        </button>
      {:else if kind === 'export_to_person' && !savedFilename}
        <button
          type="button"
          class="btn-hero"
          disabled={!preview || saving || Boolean(loadError)}
          onclick={handleSave}
        >
          {SAVE_REDACTED_PDF_LABEL}
        </button>
      {/if}
    </div>
  </header>

  <div class="tabs-bar">
    <button
      type="button"
      class="tab"
      class:active={kind === 'export_to_person'}
      onclick={() => switchKind('export_to_person')}
    >
      {EXPORT_PDF_LABEL}
    </button>
    <button
      type="button"
      class="tab"
      class:active={kind === 'share_to_ai'}
      onclick={() => switchKind('share_to_ai')}
    >
      {ASK_CLOUD_AI_LABEL}
    </button>
  </div>

  {#if loadError}
    <p class="notice error" role="alert">{loadError}</p>
    {#if loadError === CLOUD_AI_NOT_CONFIGURED_COPY}
      <p class="notice">
        <button type="button" class="btn-outlined" onclick={onNavigateSettings}>
          {OPEN_SETTINGS_LABEL}
        </button>
      </p>
    {/if}
  {:else if savedFilename}
    <p class="notice" role="status">
      Saved {savedFilename}
      {#if savedPath}
        <span class="path">{savedPath}</span>
      {/if}
    </p>
  {:else if aiOutput}
    <section class="preview-pane">
      <p class="eyebrow">Model output</p>
      <pre class="payload">{aiOutput}</pre>
    </section>
  {:else if kind === 'share_to_ai' && !preview}
    <section class="preview-pane ai-form">
      <label>
        Instruction
        <textarea bind:value={aiInstruction} maxlength="4000" rows="4"></textarea>
      </label>
      <button
        type="button"
        class="btn-hero"
        disabled={aiInstruction.trim().length === 0}
        onclick={() => loadPreview()}
      >
        {AI_PREVIEW_LABEL}
      </button>
    </section>
  {:else if preview}
    <div class="panes">
      <section class="preview-pane" aria-label={kind === 'share_to_ai' ? 'AI payload preview' : 'Redacted PDF preview'}>
        {#if preview.overrides_in_effect}
          <p class="warning" role="status">{EPHEMERAL_OVERRIDE_COPY}</p>
        {/if}
        {#if kind === 'share_to_ai'}
          <p class="warning" role="status">{AI_CONFIRM_COPY}</p>
          <pre class="payload" aria-readonly="true">{preview.ai_payload_preview}</pre>
        {:else if blobUrl}
          <iframe title="Redacted PDF preview" src={blobUrl}></iframe>
        {/if}
      </section>
      <aside class="manifest-pane">
        <p class="eyebrow">What's in this export</p>
        {#each preview.manifest as entry (entry.doc_id)}
          <p class="manifest-heading">Kept, visible</p>
          <ul class="field-list">
            {#each entry.visible_field_ids as id (id)}
              <li>
                <span>{id}</span>
                <KeepRedactControl
                  value={decisionFor(id, true)}
                  onChange={(d) => handleFieldDecision(id, d)}
                />
              </li>
            {/each}
          </ul>
          <p class="manifest-heading">Redacted</p>
          <ul class="field-list">
            {#each entry.redacted_field_ids as id (id)}
              <li>
                <span>{id}</span>
                <KeepRedactControl
                  value={decisionFor(id, false)}
                  onChange={(d) => handleFieldDecision(id, d)}
                />
              </li>
            {/each}
          </ul>
        {/each}

        {#if variants.length > 0}
          <label class="variant-apply">
            Apply a variant
            <select
              value={appliedVariantId}
              onchange={(e) => {
                appliedVariantId = (e.currentTarget as HTMLSelectElement).value
                void loadPreview()
              }}
            >
              <option value="">None</option>
              {#each variants as variant (variant.variant_id)}
                <option value={variant.variant_id}>{variant.name}</option>
              {/each}
            </select>
          </label>
        {/if}

        <div class="save-variant">
          <label>
            Variant name
            <input bind:value={variantName} maxlength="80" />
          </label>
          <button
            type="button"
            class="btn-outlined"
            disabled={variantName.trim().length === 0}
            onclick={handleSaveVariant}
          >
            {SAVE_VARIANT_LABEL}
          </button>
          {#if variantStatus}
            <p class="notice" role="status">{variantStatus}</p>
          {/if}
        </div>
      </aside>
    </div>
  {/if}

  {#if writeFailed}
    <p class="notice error" role="alert">{SHARE_WRITE_FAILED_COPY}</p>
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

  .tabs-bar {
    display: flex;
    gap: 8px;
    padding: 12px 24px;
    border-bottom: 1px solid var(--md-outline-variant);
  }

  .tab {
    height: 36px;
    padding: 0 18px;
    border: none;
    border-radius: var(--md-radius-full);
    background: transparent;
    color: var(--md-on-surface-variant);
    font-size: 13.5px;
    font-weight: 500;
    cursor: pointer;
  }

  .tab.active {
    background: var(--md-secondary-container);
    color: var(--md-on-secondary-container);
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

  .preview-pane {
    flex: 1;
    overflow: auto;
    padding: 28px 32px;
    background: var(--md-surface-container-low);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
  }

  .ai-form {
    align-items: stretch;
    max-width: 640px;
    width: 100%;
    margin: 0 auto;
  }

  .ai-form label,
  .save-variant label,
  .variant-apply {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
    font-size: 12px;
    color: var(--md-on-surface-variant);
  }

  textarea,
  input,
  select {
    font: inherit;
    padding: 8px 12px;
    border-radius: var(--md-radius-xs);
    border: 1px solid var(--md-outline);
    background: var(--md-surface-container-lowest);
    color: var(--md-on-surface);
  }

  .warning {
    margin: 0;
    max-width: 640px;
    width: 100%;
    padding: 14px 16px;
    border-radius: var(--md-radius-md);
    background: var(--md-warning-container);
    color: var(--md-on-warning-container);
    font-size: 14px;
    line-height: 20px;
  }

  iframe,
  .payload {
    width: 100%;
    max-width: 640px;
    min-height: 240px;
    border: none;
    background: var(--md-surface-container-lowest);
    border-radius: var(--md-radius-xs);
  }

  .payload {
    padding: 16px;
    font-size: 13px;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow: auto;
    box-sizing: border-box;
  }

  .manifest-pane {
    width: 340px;
    min-width: 240px;
    border-left: 1px solid var(--md-outline-variant);
    overflow: auto;
    padding: 20px;
  }

  .eyebrow {
    margin: 0 0 12px;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.4px;
    text-transform: uppercase;
    color: var(--md-on-surface-variant);
  }

  .manifest-heading {
    margin: 0 0 6px;
    font-size: 14px;
    font-weight: 500;
  }

  .field-list {
    margin: 0 0 16px;
    padding: 0;
    list-style: none;
    font-size: 14px;
  }

  .field-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 0;
  }

  .save-variant {
    border-top: 1px solid var(--md-outline-variant);
    padding-top: 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .variant-apply {
    margin-bottom: 16px;
  }

  .notice {
    margin: 16px 24px;
    font-size: 13px;
  }

  .notice.error {
    color: var(--md-error);
  }

  .path {
    display: block;
    margin-top: 4px;
    color: var(--md-on-surface-variant);
    font-size: 12px;
  }
</style>
