import { render, screen, fireEvent, waitFor, act } from '@testing-library/svelte'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import ApprovalScreen from './ApprovalScreen.svelte'
import type {
  ApprovalPage,
  ApprovalView,
  DetectedFieldDto,
  DocumentSummary,
  FieldDecisionKind,
} from '../lib/api'
import {
  APPROVE_AND_STORE_LABEL,
  APPROVAL_CANCEL_LABEL,
  APPROVAL_TITLE,
  KEEP_LABEL,
  REDACT_LABEL,
  ALREADY_APPROVED_COPY,
} from '../lib/copy'

const invokeMock = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

function makeField(overrides?: Partial<DetectedFieldDto> & { span?: Partial<DetectedFieldDto['span']> }): DetectedFieldDto {
  const spanOverrides = overrides?.span
  const rest = { ...overrides }
  delete rest.span
  return {
    id: 'f1',
    label: 'Name',
    classification: 'person_name',
    span: {
      byte_offset: 6,
      byte_length: 4,
      text: 'Rosa',
      page_index: 0,
      ...spanOverrides,
    },
    parent_field_id: null,
    ...rest,
  }
}

function pageWithText(text: string, pageIndex = 0): ApprovalPage {
  return {
    page_index: pageIndex,
    spans: [{ byte_offset: 0, text, page_index: pageIndex }],
  }
}

function makeView(overrides?: Partial<ApprovalView>): ApprovalView {
  return {
    approval_session_id: 'sess-1',
    doc_id: 'doc-1',
    lifecycle: 'awaiting_decisions',
    pages: [pageWithText('Hello Rosa here')],
    fields: [makeField()],
    ...overrides,
  }
}

function approvedSummary(): DocumentSummary {
  return {
    doc_id: 'doc-1',
    source_filename: 'letter.txt',
    source_format: 'text',
    imported_at: '2026-08-24T10:00:00Z',
    retention: 'discard',
    has_approved_version: true,
    has_retained_original: false,
    detected_field_count: 1,
  }
}

function mockApproval(view: ApprovalView, options?: { decisions?: Record<string, FieldDecisionKind> }) {
  const decisions = options?.decisions ?? {}
  invokeMock.mockImplementation((cmd: string, args?: unknown) => {
    if (cmd === 'open_approval') {
      return Promise.resolve(view)
    }
    if (cmd === 'set_field_decisions') {
      const input = (args as { input: { decisions: { field_id: string; decision: FieldDecisionKind }[] } })
        .input
      for (const d of input.decisions) {
        decisions[d.field_id] = d.decision
      }
      const unresolved = view.fields.filter((f) => !(f.id in decisions)).map((f) => f.id)
      return Promise.resolve({
        lifecycle: unresolved.length === 0 ? 'decided' : 'awaiting_decisions',
        unresolved_field_ids: unresolved,
      })
    }
    if (cmd === 'submit_approval') {
      return Promise.resolve({ summary: approvedSummary(), lifecycle: 'committed' })
    }
    if (cmd === 'abort_approval') {
      return Promise.resolve({ lifecycle: 'aborted' })
    }
    return Promise.reject(new Error(`unexpected command: ${cmd}`))
  })
}

function baseProps(overrides?: { docId?: string; sourceFilename?: string }) {
  return {
    docId: overrides?.docId ?? 'doc-1',
    sourceFilename: overrides?.sourceFilename ?? 'letter.txt',
    onLock: vi.fn(),
    onNavigateVault: vi.fn(),
    onNavigateSettings: vi.fn(),
    onNavigateAudit: vi.fn(),
    onDone: vi.fn(),
  }
}

async function loaded() {
  await screen.findByLabelText('Document text')
}

beforeEach(() => {
  invokeMock.mockReset()
})

describe('ApprovalScreen — Approve disabled until decided (ui.md §8, FR-3.1)', () => {
  it('disables Approve and store while lifecycle is awaiting_decisions', async () => {
    mockApproval(
      makeView({
        fields: [
          makeField({ id: 'f1', label: 'Name' }),
          makeField({
            id: 'f2',
            label: 'Email',
            classification: 'email',
            span: { byte_offset: 11, byte_length: 4, text: 'here', page_index: 0 },
          }),
        ],
      }),
    )
    render(ApprovalScreen, baseProps())

    await loaded()
    const approve = screen.getByRole('button', { name: APPROVE_AND_STORE_LABEL })
    expect(approve).toBeDisabled()
    expect(invokeMock).toHaveBeenCalledWith(
      'open_approval',
      expect.objectContaining({ input: { doc_id: 'doc-1' } }),
    )
  })

  it('enables Approve and store only after every field has a decision, then submit_approval fires', async () => {
    mockApproval(
      makeView({
        fields: [
          makeField({ id: 'f1', label: 'Name' }),
          makeField({
            id: 'f2',
            label: 'Email',
            classification: 'email',
            span: { byte_offset: 11, byte_length: 4, text: 'here', page_index: 0 },
          }),
        ],
      }),
    )
    const props = baseProps()
    render(ApprovalScreen, props)

    await loaded()
    const approve = screen.getByRole('button', { name: APPROVE_AND_STORE_LABEL })
    expect(approve).toBeDisabled()

    const keepButtons = screen.getAllByRole('button', { name: KEEP_LABEL })
    await fireEvent.click(keepButtons[0])
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        'set_field_decisions',
        expect.objectContaining({
          input: {
            approval_session_id: 'sess-1',
            decisions: [{ field_id: 'f1', decision: 'keep_visible' }],
          },
        }),
      )
    })
    expect(approve).toBeDisabled()

    const redactButtons = screen.getAllByRole('button', { name: REDACT_LABEL })
    await fireEvent.click(redactButtons[1])
    await waitFor(() => {
      expect(approve).toBeEnabled()
    })

    await fireEvent.click(approve)
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        'submit_approval',
        expect.objectContaining({ input: { approval_session_id: 'sess-1' } }),
      )
    })
    expect(props.onDone).toHaveBeenCalled()
  })
})

describe('ApprovalScreen — first paint (ui.md §14, §16)', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('paints first page text and the first 200 field rows within 300ms of open_approval resolving', async () => {
    vi.useFakeTimers()
    const fields: DetectedFieldDto[] = Array.from({ length: 250 }, (_, i) =>
      makeField({
        id: `f-${i}`,
        label: `Field ${i}`,
        classification: 'other',
        span: { byte_offset: 0, byte_length: 5, text: 'Hello', page_index: 0 },
      }),
    )
    mockApproval(
      makeView({
        pages: [pageWithText('PAGE ONE BODY TEXT')],
        fields,
      }),
    )

    render(ApprovalScreen, baseProps())

    await act(async () => {
      await Promise.resolve()
      await Promise.resolve()
    })

    expect(screen.getByLabelText('Document text')).toHaveTextContent('PAGE ONE BODY TEXT')
    expect(screen.getByText('Field 0')).toBeInTheDocument()
    expect(screen.getByText('Field 199')).toBeInTheDocument()
    // Progressive list: do not block first paint on the full 250 (ui.md §8).
    expect(screen.queryByText('Field 200')).not.toBeInTheDocument()

    await act(async () => {
      vi.advanceTimersByTime(300)
    })
  })
})

describe('ApprovalScreen — keyboard (ui.md §16, §19)', () => {
  it('field list and keep/redact are operable without a pointer', async () => {
    const user = userEvent.setup()
    mockApproval(
      makeView({
        fields: [
          makeField({ id: 'f1', label: 'Name' }),
          makeField({
            id: 'f2',
            label: 'Email',
            classification: 'email',
            span: { byte_offset: 11, byte_length: 4, text: 'here', page_index: 0 },
          }),
        ],
      }),
    )
    render(ApprovalScreen, baseProps())
    await loaded()

    const keepButtons = screen.getAllByRole('button', { name: KEEP_LABEL })
    keepButtons[0].focus()
    expect(keepButtons[0]).toHaveFocus()
    await user.keyboard('{Enter}')
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        'set_field_decisions',
        expect.objectContaining({
          input: expect.objectContaining({
            decisions: [{ field_id: 'f1', decision: 'keep_visible' }],
          }),
        }),
      )
    })

    const redactButtons = screen.getAllByRole('button', { name: REDACT_LABEL })
    redactButtons[1].focus()
    await user.keyboard(' ')
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        'set_field_decisions',
        expect.objectContaining({
          input: expect.objectContaining({
            decisions: [{ field_id: 'f2', decision: 'redact' }],
          }),
        }),
      )
    })

    const nameRow = screen.getByRole('button', { name: 'Name' })
    const emailRow = screen.getByRole('button', { name: 'Email' })
    nameRow.focus()
    await user.keyboard('{ArrowDown}')
    expect(emailRow).toHaveAttribute('aria-pressed', 'true')
  })
})

describe('ApprovalScreen — layout and locatable spans (ui.md §8, NFR-U2, FR-2.2)', () => {
  it('renders two panes with page text and a field row per DetectedFieldDto, keep/redact not colour-only', async () => {
    mockApproval(makeView())
    render(ApprovalScreen, baseProps())

    expect(await screen.findByRole('heading', { name: APPROVAL_TITLE })).toBeInTheDocument()
    await loaded()
    expect(screen.getByLabelText('Document text')).toHaveTextContent('Hello Rosa here')
    expect(screen.getByText('Name')).toBeInTheDocument()
    expect(screen.getByText('person_name')).toBeInTheDocument()
    // NFR-U2: the words themselves, not a colour-only control.
    expect(screen.getByRole('button', { name: KEEP_LABEL })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: REDACT_LABEL })).toBeInTheDocument()
    expect(screen.getByText('letter.txt')).toBeInTheDocument()
  })

  it('does not hide a nested field inside its parent (design §3.5)', async () => {
    mockApproval(
      makeView({
        pages: [pageWithText('Address 62704 end')],
        fields: [
          makeField({
            id: 'outer',
            label: 'Address',
            classification: 'address',
            span: { byte_offset: 0, byte_length: 15, text: 'Address 62704', page_index: 0 },
          }),
          makeField({
            id: 'inner',
            label: 'Postal code',
            classification: 'postal_code',
            parent_field_id: 'outer',
            span: { byte_offset: 8, byte_length: 5, text: '62704', page_index: 0 },
          }),
        ],
      }),
    )
    render(ApprovalScreen, baseProps())

    expect(await screen.findByText('Postal code')).toBeInTheDocument()
    expect(screen.getAllByText(/Address/).length).toBeGreaterThan(0)
    expect(screen.getByText('62704')).toBeInTheDocument()
  })

  it('selecting a list row highlights the matching span and vice versa', async () => {
    mockApproval(makeView())
    render(ApprovalScreen, baseProps())
    await loaded()
    await screen.findByText('Name')

    const option = screen.getByRole('button', { name: 'Name' })
    await fireEvent.click(option)
    const span = screen.getByTestId('field-span-f1')
    expect(span).toHaveAttribute('data-selected', 'true')

    await fireEvent.click(span)
    expect(option).toHaveAttribute('aria-pressed', 'true')
  })

  it('does not offer a download of the original', async () => {
    mockApproval(makeView())
    render(ApprovalScreen, baseProps())
    await screen.findByRole('heading', { name: APPROVAL_TITLE })
    expect(screen.queryByRole('button', { name: /download/i })).not.toBeInTheDocument()
    expect(screen.queryByText(/download/i)).not.toBeInTheDocument()
  })
})

describe('ApprovalScreen — cancel and errors (ui.md §8, api.md §5.4)', () => {
  it('Cancel calls abort_approval and returns via onDone', async () => {
    mockApproval(makeView())
    const props = baseProps()
    render(ApprovalScreen, props)
    await loaded()
    await fireEvent.click(screen.getByRole('button', { name: APPROVAL_CANCEL_LABEL }))
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        'abort_approval',
        expect.objectContaining({ input: { approval_session_id: 'sess-1' } }),
      )
    })
    expect(props.onDone).toHaveBeenCalled()
  })

  it('surfaces already_approved from open_approval and does not enable Approve and store', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'open_approval') {
        return Promise.reject({
          code: 'already_approved',
          message: 'document already has an approved version',
        })
      }
      if (cmd === 'abort_approval') {
        return Promise.resolve({ lifecycle: 'aborted' })
      }
      return Promise.reject(new Error(`unexpected command: ${cmd}`))
    })
    render(ApprovalScreen, baseProps())
    expect(await screen.findByText(ALREADY_APPROVED_COPY)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: APPROVE_AND_STORE_LABEL })).not.toBeInTheDocument()
  })
})
