<script lang="ts">
  // ui.md §12 — audit table. Unlocked only (degraded is the §13 full-screen). Filter by
  // document and event type; share rows answer "what did I share, and to whom?" at NFR-U2
  // reading level. Field/span text never appears (C-UI-2 / C-API-2).

  import { onMount } from 'svelte'
  import AppShell from '../lib/AppShell.svelte'
  import {
    listAuditEvents,
    listDocuments,
    type AuditEventDto,
    type DocumentSummary,
    type EventType,
  } from '../lib/api'
  import {
    AUDIT_EMPTY_FILTER_COPY,
    AUDIT_EVENT_TYPE_LABELS,
    AUDIT_ORIGINALS_GONE_COPY,
    AUDIT_ORIGINALS_KEPT_COPY,
    AUDIT_SHARE_AI_LABEL,
    AUDIT_SHARE_EXPORT_LABEL,
    AUDIT_TITLE,
  } from '../lib/copy'

  let {
    onLock,
    onNavigateVault,
    onNavigateSettings,
  }: {
    onLock: () => void
    onNavigateVault: () => void
    onNavigateSettings: () => void
  } = $props()

  const PAGE_SIZE = 50
  const EVENT_TYPES: EventType[] = [
    'import',
    'detect',
    'approve',
    'share',
    'discard_original',
    'delete',
  ]

  let documents = $state<DocumentSummary[]>([])
  let events = $state<AuditEventDto[]>([])
  let nextSequence = $state<number | null>(null)
  let docFilter = $state('')
  let typeFilter = $state('')
  let loadError = $state('')
  let loading = $state(true)

  const filenameById = $derived(new Map(documents.map((d) => [d.doc_id, d.source_filename])))

  function payloadString(payload: Record<string, unknown>, key: string): string | null {
    const value = payload[key]
    return typeof value === 'string' && value.length > 0 ? value : null
  }

  function eventLabel(event: AuditEventDto): string {
    if (event.event_type === 'share') {
      return event.payload.kind === 'share_to_ai' ? AUDIT_SHARE_AI_LABEL : AUDIT_SHARE_EXPORT_LABEL
    }
    return AUDIT_EVENT_TYPE_LABELS[event.event_type]
  }

  function documentLabel(event: AuditEventDto): string {
    const fromImport = payloadString(event.payload, 'source_filename')
    if (fromImport) return fromImport
    const docIds = event.payload.doc_ids
    if (Array.isArray(docIds) && docIds.length > 0) {
      return docIds
        .map((id) => (typeof id === 'string' ? (filenameById.get(id) ?? id) : ''))
        .filter(Boolean)
        .join(', ')
    }
    if (event.doc_id) return filenameById.get(event.doc_id) ?? event.doc_id
    return '–'
  }

  function destinationLabel(event: AuditEventDto): string {
    return (
      payloadString(event.payload, 'recipient_note') ??
      payloadString(event.payload, 'endpoint_host') ??
      '–'
    )
  }

  function shareKindLabel(event: AuditEventDto): string {
    if (event.event_type !== 'share') return '–'
    return event.payload.kind === 'share_to_ai' ? 'Ask AI' : 'Export'
  }

  function originalsLabel(event: AuditEventDto): string {
    if (event.no_originals_left_device === true) return AUDIT_ORIGINALS_GONE_COPY
    if (event.no_originals_left_device === false) return AUDIT_ORIGINALS_KEPT_COPY
    return '–'
  }

  function formatTime(iso: string): string {
    const parsed = new Date(iso)
    if (Number.isNaN(parsed.getTime())) return iso
    return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(
      parsed,
    )
  }

  async function loadEvents(append: boolean) {
    loading = true
    loadError = ''
    try {
      const out = await listAuditEvents({
        doc_id: docFilter || null,
        event_type: (typeFilter || null) as EventType | null,
        after_sequence: append ? nextSequence : null,
        limit: PAGE_SIZE,
      })
      events = append ? [...events, ...out.events] : out.events
      nextSequence = out.next_sequence
    } catch (err) {
      loadError = err instanceof Error ? err.message : 'Could not load the audit trail.'
    } finally {
      loading = false
    }
  }

  async function handleDocFilter(event: Event) {
    docFilter = (event.currentTarget as HTMLSelectElement).value
    nextSequence = null
    await loadEvents(false)
  }

  async function handleTypeFilter(event: Event) {
    typeFilter = (event.currentTarget as HTMLSelectElement).value
    nextSequence = null
    await loadEvents(false)
  }

  onMount(() => {
    void (async () => {
      try {
        documents = (await listDocuments()).documents
      } catch {
        documents = []
      }
      await loadEvents(false)
    })()
  })
</script>

<div class="screen">
  <AppShell
    active="audit"
    {onNavigateVault}
    onNavigateAudit={() => {}}
    {onNavigateSettings}
    {onLock}
  />

  <header class="topbar">
    <h1>{AUDIT_TITLE}</h1>
    <div class="filters">
      <label>
        Document
        <select value={docFilter} onchange={handleDocFilter}>
          <option value="">All</option>
          {#each documents as doc (doc.doc_id)}
            <option value={doc.doc_id}>{doc.source_filename}</option>
          {/each}
        </select>
      </label>
      <label>
        Event type
        <select value={typeFilter} onchange={handleTypeFilter}>
          <option value="">All</option>
          {#each EVENT_TYPES as type (type)}
            <option value={type}>{AUDIT_EVENT_TYPE_LABELS[type]}</option>
          {/each}
        </select>
      </label>
    </div>
  </header>

  {#if loadError}
    <p class="notice" role="alert">{loadError}</p>
  {:else if loading && events.length === 0}
    <!-- First fetch in flight — do not flash the empty-filter copy. -->
  {:else if events.length === 0}
    <p class="notice" role="status">{AUDIT_EMPTY_FILTER_COPY}</p>
  {:else}
    <div class="table-wrap">
      <table>
        <thead>
          <tr>
            <th>Time</th>
            <th>Event</th>
            <th>Document</th>
            <th>Destination</th>
            <th>Kind</th>
            <th>Originals</th>
          </tr>
        </thead>
        <tbody>
          {#each events as event (event.sequence)}
            <tr>
              <td>{formatTime(event.produced_at)}</td>
              <td><span class="chip">{eventLabel(event)}</span></td>
              <td>{documentLabel(event)}</td>
              <td>{destinationLabel(event)}</td>
              <td>{shareKindLabel(event)}</td>
              <td>{originalsLabel(event)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    {#if nextSequence !== null}
      <div class="more">
        <button type="button" class="btn-outlined" disabled={loading} onclick={() => loadEvents(true)}>
          Load more
        </button>
      </div>
    {/if}
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

  h1 {
    margin: 0;
    font-size: 24px;
    line-height: 32px;
    font-weight: 400;
  }

  .filters {
    display: flex;
    gap: 8px;
  }

  label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--md-on-surface-variant);
  }

  select {
    height: 36px;
    padding: 0 12px;
    border-radius: var(--md-radius-full);
    border: 1px solid var(--md-outline);
    background: transparent;
    color: var(--md-on-surface);
    font: inherit;
  }

  .table-wrap {
    flex: 1;
    overflow: auto;
    padding: 24px 32px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  th {
    text-align: left;
    padding: 12px 16px;
    background: var(--md-surface-container-low);
    color: var(--md-on-surface-variant);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.5px;
    text-transform: uppercase;
    border-bottom: 1px solid var(--md-outline-variant);
  }

  td {
    padding: 13px 16px;
    border-bottom: 1px solid var(--md-outline-variant);
    font-size: 13.5px;
    color: var(--md-on-surface);
    vertical-align: top;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    height: 24px;
    padding: 0 10px;
    border-radius: var(--md-radius-full);
    font-size: 11px;
    font-weight: 500;
    background: var(--md-surface-container-low);
    color: var(--md-on-surface-variant);
    border: 1px solid var(--md-outline);
  }

  .notice {
    margin: 24px 32px;
    font-size: 14px;
    color: var(--md-on-surface-variant);
  }

  .more {
    padding: 0 32px 24px;
  }

  .btn-outlined {
    height: 36px;
    padding: 0 16px;
    border-radius: var(--md-radius-full);
    border: 1px solid var(--md-outline);
    background: transparent;
    color: var(--md-on-surface-variant);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
  }

  .btn-outlined:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
