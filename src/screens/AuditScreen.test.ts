import { render, screen, fireEvent, waitFor, within } from '@testing-library/svelte'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import AuditScreen from './AuditScreen.svelte'
import type { AuditEventDto, DocumentSummary } from '../lib/api'
import {
  AUDIT_EMPTY_FILTER_COPY,
  AUDIT_ORIGINALS_GONE_COPY,
  AUDIT_ORIGINALS_KEPT_COPY,
  AUDIT_SHARE_AI_LABEL,
  AUDIT_SHARE_EXPORT_LABEL,
  INTEGRITY_BODY,
  INTEGRITY_TITLE,
} from '../lib/copy'

const invokeMock = vi.hoisted(() => vi.fn())

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

const DOC: DocumentSummary = {
  doc_id: 'doc-1',
  source_filename: 'letter.txt',
  source_format: 'text',
  imported_at: '2026-08-20T08:51:00Z',
  retention: 'discard',
  has_approved_version: true,
  has_retained_original: false,
  detected_field_count: 1,
}

function makeEvent(overrides?: Partial<AuditEventDto>): AuditEventDto {
  return {
    sequence: 1,
    event_type: 'import',
    doc_id: 'doc-1',
    produced_at: '2026-08-20T08:51:00Z',
    no_originals_left_device: null,
    payload: { retention: 'discard', source_filename: 'letter.txt', detector_id: null },
    ...overrides,
  }
}

const SHARE_EXPORT = makeEvent({
  sequence: 4,
  event_type: 'share',
  produced_at: '2026-08-23T09:14:00Z',
  no_originals_left_device: true,
  payload: {
    kind: 'export_to_person',
    recipient_note: 'For underwriting review',
    endpoint_host: null,
    doc_ids: ['doc-1'],
    error_class: null,
    has_ai_instruction: false,
  },
})

const SHARE_AI = makeEvent({
  sequence: 5,
  event_type: 'share',
  produced_at: '2026-08-22T16:02:00Z',
  no_originals_left_device: false,
  payload: {
    kind: 'share_to_ai',
    recipient_note: null,
    endpoint_host: 'api.reviewassist.example.com',
    doc_ids: ['doc-1'],
    error_class: null,
    has_ai_instruction: true,
  },
})

function mockList(events: AuditEventDto[], next_sequence: number | null = null) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'list_documents') return Promise.resolve({ documents: [DOC] })
    if (cmd === 'list_audit_events') return Promise.resolve({ events, next_sequence })
    return Promise.reject(new Error(`unexpected command: ${cmd}`))
  })
}

function baseProps() {
  return {
    onLock: vi.fn(),
    onNavigateVault: vi.fn(),
    onNavigateSettings: vi.fn(),
  }
}

async function loadedTable() {
  return screen.findByRole('table')
}

async function loadedEmpty() {
  return screen.findByText(AUDIT_EMPTY_FILTER_COPY)
}

beforeEach(() => {
  invokeMock.mockReset()
})

describe('AuditScreen — table (ui.md §12, NFR-U1)', () => {
  it('renders events in words with document names, not a field-text dump', async () => {
    mockList([
      makeEvent(),
      makeEvent({
        sequence: 2,
        event_type: 'detect',
        payload: { detector_id: 'pg-hybrid-v1', field_ids: ['f1'], labels: ['Name'] },
      }),
      makeEvent({ sequence: 3, event_type: 'approve', payload: { decisions: [] } }),
      SHARE_EXPORT,
    ])
    render(AuditScreen, baseProps())
    const table = await loadedTable()

    expect(within(table).getByText('Imported')).toBeInTheDocument()
    expect(within(table).getByText('Detected')).toBeInTheDocument()
    expect(within(table).getByText('Approved')).toBeInTheDocument()
    expect(within(table).getByText(AUDIT_SHARE_EXPORT_LABEL)).toBeInTheDocument()
    expect(within(table).getAllByText('letter.txt').length).toBeGreaterThan(0)
    expect(screen.queryByText('PG-CANARY-SECRET')).not.toBeInTheDocument()
    expect(screen.queryByText(INTEGRITY_TITLE)).not.toBeInTheDocument()
    expect(screen.queryByText(INTEGRITY_BODY)).not.toBeInTheDocument()
  })

  it('share row answers what was shared and to whom (NFR-U2)', async () => {
    mockList([SHARE_EXPORT, SHARE_AI])
    render(AuditScreen, baseProps())
    const table = await loadedTable()

    expect(within(table).getByText(AUDIT_SHARE_EXPORT_LABEL)).toBeInTheDocument()
    expect(within(table).getByText('For underwriting review')).toBeInTheDocument()
    expect(within(table).getByText(AUDIT_ORIGINALS_GONE_COPY)).toBeInTheDocument()

    expect(within(table).getByText(AUDIT_SHARE_AI_LABEL)).toBeInTheDocument()
    expect(within(table).getByText('api.reviewassist.example.com')).toBeInTheDocument()
    expect(within(table).getByText(AUDIT_ORIGINALS_KEPT_COPY)).toBeInTheDocument()
  })

  it('shows the empty-filter copy when nothing matches', async () => {
    mockList([])
    render(AuditScreen, baseProps())
    await loadedEmpty()
  })

  it('filters by document and event type via list_audit_events', async () => {
    mockList([makeEvent(), SHARE_EXPORT])
    render(AuditScreen, baseProps())
    await loadedTable()

    await fireEvent.change(screen.getByLabelText('Document'), { target: { value: 'doc-1' } })
    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter((c) => c[0] === 'list_audit_events')
      expect(calls[calls.length - 1][1]).toEqual(
        expect.objectContaining({
          input: expect.objectContaining({ doc_id: 'doc-1', event_type: null }),
        }),
      )
    })

    await fireEvent.change(screen.getByLabelText('Event type'), { target: { value: 'share' } })
    await waitFor(() => {
      const calls = invokeMock.mock.calls.filter((c) => c[0] === 'list_audit_events')
      expect(calls[calls.length - 1][1]).toEqual(
        expect.objectContaining({
          input: expect.objectContaining({ doc_id: 'doc-1', event_type: 'share' }),
        }),
      )
    })
  })

  it('does not render redacted span text even if a payload smuggles it', async () => {
    mockList([
      makeEvent({
        event_type: 'detect',
        payload: {
          detector_id: 'pg-hybrid-v1',
          field_ids: ['f1'],
          labels: ['Name'],
          text: 'PG-CANARY-SECRET',
        },
      }),
    ])
    render(AuditScreen, baseProps())
    const table = await loadedTable()
    expect(screen.queryByText('PG-CANARY-SECRET')).not.toBeInTheDocument()
    expect(within(table).getByText('Detected')).toBeInTheDocument()
  })
})
