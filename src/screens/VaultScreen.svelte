<script lang="ts">
  // ui.md §6–§7 — vault list + first-import modal + import. Replaces the W30/W31
  // empty-state placeholder with the real `list_documents` read path and the
  // `import_document` write path (file input + drag-and-drop), gated by decision 0007's
  // blocking retention modal.
  //
  // §7.3 gap ("navigate to Approval on `has_approved_version === false`"): the Approval
  // screen is W33, not built yet. This screen does **not** fake or partially build it.
  // Instead: a freshly-imported (or any unapproved) row's "Open" action shows an inline
  // "Approval screen not yet available" placeholder rather than navigating anywhere or
  // silently doing nothing — see docs/dev-log/0044-w32-ui-vault-import.md.
  //
  // C-UI-1 / architecture §12: import reads `File` bytes in memory (`file.arrayBuffer()`)
  // and never touches `@tauri-apps/plugin-fs` or any filesystem-path API — this file has
  // no such import, by construction, not merely by convention.

  import { onMount } from 'svelte'
  import { listen, type UnlistenFn } from '@tauri-apps/api/event'
  import AppShell from '../lib/AppShell.svelte'
  import RetentionModal from '../lib/RetentionModal.svelte'
  import RetentionOverrideControl from '../lib/RetentionOverrideControl.svelte'
  import {
    DETECT_PROGRESS_EVENT,
    deleteDocument,
    getRetentionDefault,
    importDocument,
    isApiError,
    listDocuments,
    setRetentionDefault,
    type DetectProgressEvent,
    type DocumentSummary,
    type EffectiveRetention,
    type RetentionPolicy,
  } from '../lib/api'
  import {
    IMPORT_INVALID_INPUT_COPY,
    OVER_BUDGET_COPY,
    RETENTION_POLICY_UNSET_COPY,
    RETENTION_LOOSEN_FORBIDDEN_COPY,
    UNSUPPORTED_DOCUMENT_COPY,
    DELETE_DOCUMENT_CONFIRM_COPY,
    VAULT_EMPTY_STATE_COPY,
  } from '../lib/copy'

  let { onLock, onNavigateSettings }: { onLock: () => void; onNavigateSettings: () => void } =
    $props()

  // --- Vault list (§7.1) --------------------------------------------------
  let documents = $state<DocumentSummary[]>([])
  let listLoaded = $state(false)

  async function refreshList() {
    const out = await listDocuments()
    documents = out.documents
    listLoaded = true
  }

  // --- Retention gate (§6, decision 0007) ---------------------------------
  let retentionConfirmed = $state(false)
  let retentionDefaultPolicy = $state<RetentionPolicy>('discard')
  let showRetentionModal = $state(false)
  // A drop that arrives before the default is confirmed is stashed here so the flow
  // doesn't force the user to re-drag once they've confirmed the modal (§6 step 4/5: the
  // drop is only *accepted* — i.e. acted on — after Continue, not discarded).
  let pendingDropFile = $state<File | null>(null)

  // --- Per-import override (§7.2 "Later imports") -------------------------
  let overrideValue = $state<EffectiveRetention | null>(null)

  // --- Import in flight -----------------------------------------------------
  let fileInputEl = $state<HTMLInputElement | undefined>(undefined)
  let importing = $state(false)
  let progressFraction = $state(0)
  let importError = $state('')
  let overBudgetNotice = $state(false)
  let dragOver = $state(false)
  // §7.3 gap-handling: the doc_id of the row we should flag "Open" as deferred for,
  // distinctly from any other unapproved row already in the vault (all unapproved rows get
  // the same placeholder — this just tracks "most recently imported" for no special
  // treatment beyond that; kept for clarity/debuggability, not behavior).
  let lastImportedDocId = $state<string | null>(null)

  // --- Row-level "Open" placeholder (§7.3 gap) -----------------------------
  let openPlaceholderDocId = $state<string | null>(null)

  // --- Delete confirm (mirrors SettingsScreen's Cloud-AI-clear pattern) ---
  let deleteConfirmDocId = $state<string | null>(null)
  let deleting = $state(false)

  function requestImport() {
    importError = ''
    if (!retentionConfirmed) {
      showRetentionModal = true
      return
    }
    fileInputEl?.click()
  }

  function handleFileInputChange(event: Event) {
    const input = event.currentTarget as HTMLInputElement
    const file = input.files?.[0]
    input.value = ''
    if (file) {
      void startImport(file)
    }
  }

  function handleDragOver(event: DragEvent) {
    event.preventDefault()
    dragOver = true
  }

  function handleDragLeave() {
    dragOver = false
  }

  function handleDrop(event: DragEvent) {
    event.preventDefault()
    dragOver = false
    const file = event.dataTransfer?.files?.[0]
    if (!file) return
    importError = ''
    if (!retentionConfirmed) {
      pendingDropFile = file
      showRetentionModal = true
      return
    }
    void startImport(file)
  }

  async function handleModalContinue(policy: RetentionPolicy) {
    // dev-plan W32's explicitly-named test: "Continue sets policy before import" — await
    // `set_retention_default` (and update local state from *its* response) before doing
    // anything else, so call order is provably policy-then-import, not merely
    // both-eventually-called.
    const out = await setRetentionDefault(policy)
    retentionConfirmed = out.confirmed
    retentionDefaultPolicy = out.policy
    showRetentionModal = false

    const queued = pendingDropFile
    pendingDropFile = null
    if (queued) {
      await startImport(queued)
    } else {
      // §6 step 4: "Only then open the HTML file picker" — the Import button/affordance
      // triggered this modal with no file yet in hand.
      fileInputEl?.click()
    }
  }

  function handleModalCancel() {
    // §6 / C-UI-4: Cancel leaves `confirmed` false and does not import.
    showRetentionModal = false
    pendingDropFile = null
  }

  function basenameIsValid(name: string): boolean {
    return name.length > 0 && !name.includes('/') && !name.includes('\\')
  }

  // `File.prototype.arrayBuffer` is not implemented by jsdom's `File` (only Node's global
  // `File`, which the test environment's `File` constructor is not, per jsdom's own poly),
  // so this reads via `FileReader` instead — the one in-memory read path both jsdom and a
  // real webview support, and it never touches the filesystem or `plugin-fs` (C-UI-1).
  function readFileAsArrayBuffer(file: File): Promise<ArrayBuffer> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader()
      reader.onload = () => resolve(reader.result as ArrayBuffer)
      reader.onerror = () => reject(reader.error ?? new Error('failed to read file'))
      reader.readAsArrayBuffer(file)
    })
  }

  async function startImport(file: File) {
    importError = ''
    overBudgetNotice = false

    // Client-side defense-in-depth (ui.md §7.2): the core's own `validate_import_filename`
    // (core/src/session.rs) already rejects an empty or separator-containing filename with
    // `invalid_input`, so this check's job is to fail fast and keep the picker/drop state
    // intact rather than round-trip to the core for something detectable in the browser —
    // it is not the only guard.
    if (!basenameIsValid(file.name)) {
      importError = IMPORT_INVALID_INPUT_COPY
      return
    }

    importing = true
    progressFraction = 0
    let unlisten: UnlistenFn | undefined
    try {
      unlisten = await listen<DetectProgressEvent>(DETECT_PROGRESS_EVENT, (event) => {
        progressFraction = event.payload.fraction
      }).catch(() => undefined)

      const buffer = await readFileAsArrayBuffer(file)
      const bytes = Array.from(new Uint8Array(buffer))
      const out = await importDocument({
        filename: file.name,
        bytes,
        retention_override: overrideValue,
      })
      if (out.over_budget) {
        overBudgetNotice = true
      }
      lastImportedDocId = out.summary.doc_id
      await refreshList()
    } catch (err) {
      importError = mapImportError(err)
    } finally {
      importing = false
      unlisten?.()
    }
  }

  function mapImportError(err: unknown): string {
    if (isApiError(err)) {
      switch (err.code) {
        case 'unsupported_document':
          return UNSUPPORTED_DOCUMENT_COPY
        case 'retention_policy_unset':
          return RETENTION_POLICY_UNSET_COPY
        case 'retention_loosen_forbidden':
          return RETENTION_LOOSEN_FORBIDDEN_COPY
        case 'invalid_input':
          return IMPORT_INVALID_INPUT_COPY
        default:
          return err.message
      }
    }
    return 'Could not import this file.'
  }

  function handleOpen(docId: string) {
    // §7.3 gap-handling: no Approval screen exists yet (W33). Show a clearly-labeled
    // deferred placeholder instead of navigating nowhere or faking one.
    openPlaceholderDocId = openPlaceholderDocId === docId ? null : docId
  }

  function requestDelete(docId: string) {
    deleteConfirmDocId = docId
  }

  function cancelDelete() {
    deleteConfirmDocId = null
  }

  async function confirmDelete(docId: string) {
    deleting = true
    try {
      await deleteDocument(docId)
      await refreshList()
    } finally {
      deleting = false
      deleteConfirmDocId = null
    }
  }

  function formatImportedAt(iso: string): string {
    const parsed = new Date(iso)
    if (Number.isNaN(parsed.getTime())) return iso
    return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(
      parsed,
    )
  }

  onMount(() => {
    void refreshList()
    getRetentionDefault().then((out) => {
      retentionConfirmed = out.confirmed
      retentionDefaultPolicy = out.policy
    })
  })
</script>

<div class="screen">
  <AppShell active="vault" onNavigateVault={() => {}} {onNavigateSettings} {onLock} />

  <main>
    <section class="import-section">
      <div
        class="dropzone"
        role="group"
        aria-label="Import a document by drag-and-drop or file picker"
        class:drag-over={dragOver}
        ondragover={handleDragOver}
        ondragleave={handleDragLeave}
        ondrop={handleDrop}
      >
        <p>Drag a text file or PDF here, or</p>
        <button type="button" onclick={requestImport} disabled={importing}>
          Import a document
        </button>
        <input
          bind:this={fileInputEl}
          type="file"
          accept=".pdf,.txt,text/plain,application/pdf"
          class="visually-hidden"
          onchange={handleFileInputChange}
        />
      </div>

      {#if retentionConfirmed}
        <RetentionOverrideControl
          value={overrideValue}
          defaultIsNeverRetain={retentionDefaultPolicy === 'never_retain'}
          onChange={(next) => (overrideValue = next)}
        />
      {/if}

      {#if importing}
        <div class="progress-row">
          <progress value={progressFraction} max="1"></progress>
          <span>{Math.round(progressFraction * 100)}%</span>
        </div>
      {/if}

      {#if overBudgetNotice}
        <p class="notice">{OVER_BUDGET_COPY}</p>
      {/if}

      {#if importError}
        <p class="notice error" role="alert">{importError}</p>
      {/if}
    </section>

    {#if listLoaded && documents.length === 0}
      <p class="empty-state">{VAULT_EMPTY_STATE_COPY}</p>
    {:else if documents.length > 0}
      <table class="vault-table">
        <thead>
          <tr>
            <th>Name</th>
            <th>Format</th>
            <th>Imported</th>
            <th>Retention</th>
            <th>Fields detected</th>
            <th>Approved</th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each documents as doc (doc.doc_id)}
            <tr>
              <td>{doc.source_filename}</td>
              <td>{doc.source_format}</td>
              <td>{formatImportedAt(doc.imported_at)}</td>
              <td>{doc.retention}</td>
              <td>{doc.detected_field_count}</td>
              <td>{doc.has_approved_version ? 'Yes' : 'No'}</td>
              <td class="actions-cell">
                <button type="button" onclick={() => handleOpen(doc.doc_id)}>Open</button>
                {#if deleteConfirmDocId === doc.doc_id}
                  <span class="confirm-delete">
                    {DELETE_DOCUMENT_CONFIRM_COPY}
                    <button type="button" disabled={deleting} onclick={() => confirmDelete(doc.doc_id)}>
                      Yes, delete
                    </button>
                    <button type="button" disabled={deleting} onclick={cancelDelete}>
                      Cancel
                    </button>
                  </span>
                {:else}
                  <button type="button" onclick={() => requestDelete(doc.doc_id)}>Delete</button>
                {/if}
                {#if openPlaceholderDocId === doc.doc_id}
                  <p class="open-placeholder">Approval screen not yet available.</p>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </main>

  {#if showRetentionModal}
    <RetentionModal onContinue={handleModalContinue} onCancel={handleModalCancel} />
  {/if}
</div>

<style>
  .screen {
    min-height: 100vh;
    background: var(--md-surface);
    font-family: var(--md-font);
  }

  main {
    padding: 32px 24px;
    max-width: 900px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }

  .import-section {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .dropzone {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 32px;
    border: 1.5px dashed var(--md-outline-variant);
    border-radius: var(--md-radius-xl);
    color: var(--md-on-surface-variant);
    font-size: 13.5px;
  }

  .dropzone.drag-over {
    border-color: var(--md-primary);
    background: var(--md-surface-container-lowest);
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }

  .progress-row {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12.5px;
    color: var(--md-on-surface-variant);
  }

  progress {
    flex: 1;
    height: 6px;
  }

  .notice {
    margin: 0;
    font-size: 12.5px;
    color: var(--md-on-surface-variant);
  }

  .notice.error {
    color: var(--md-error);
  }

  .empty-state {
    color: var(--md-on-surface-variant);
    font-size: 14px;
    text-align: center;
    padding: 24px 0;
  }

  .vault-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  .vault-table th {
    text-align: left;
    font-weight: 500;
    color: var(--md-on-surface-variant);
    padding: 8px 12px;
    border-bottom: 1px solid var(--md-outline-variant);
  }

  .vault-table td {
    padding: 10px 12px;
    border-bottom: 1px solid var(--md-outline-variant);
    color: var(--md-on-surface);
  }

  .actions-cell {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .confirm-delete {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11.5px;
    color: var(--md-on-surface-variant);
  }

  .open-placeholder {
    flex-basis: 100%;
    margin: 4px 0 0;
    font-size: 11.5px;
    color: var(--md-on-surface-variant);
  }

  button {
    height: 32px;
    padding: 0 14px;
    border: none;
    border-radius: var(--md-radius-full);
    background: var(--md-primary);
    color: var(--md-on-primary);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
