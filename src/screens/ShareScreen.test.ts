import { render, screen, fireEvent, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import ShareScreen from './ShareScreen.svelte'
import type { CommitShareOut, SharePreview } from '../lib/api'
import {
  AI_CONFIRM_COPY,
  AI_PREVIEW_LABEL,
  ASK_CLOUD_AI_LABEL,
  CLOUD_AI_NOT_CONFIGURED_COPY,
  OPEN_SETTINGS_LABEL,
  EPHEMERAL_OVERRIDE_COPY,
  PREVIEW_EXPIRED_COPY,
  RETRY_SAVE_LABEL,
  SAVE_REDACTED_PDF_LABEL,
  SEND_TO_AI_LABEL,
  SHARE_TITLE,
  SHARE_WRITE_FAILED_COPY,
} from '../lib/copy'

const invokeMock = vi.hoisted(() => vi.fn())
const saveMock = vi.hoisted(() => vi.fn())
const writeFileMock = vi.hoisted(() => vi.fn())
const documentDirMock = vi.hoisted(() => vi.fn())
const joinMock = vi.hoisted(() => vi.fn())

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: saveMock }))
vi.mock('@tauri-apps/plugin-fs', () => ({ writeFile: writeFileMock }))
vi.mock('@tauri-apps/api/path', () => ({
  documentDir: documentDirMock,
  join: joinMock,
}))

const PDF_BYTES = [37, 80, 68, 70] // %PDF — not PII

function makePreview(overrides?: Partial<SharePreview>): SharePreview {
  return {
    preview_token: 'tok-1',
    expires_at: '2026-08-24T12:00:00Z',
    kind: 'export_to_person',
    overrides_in_effect: false,
    suggested_filename: 'letter-redacted.pdf',
    pdf_bytes: PDF_BYTES,
    ai_payload_preview: null,
    manifest: [
      {
        doc_id: 'doc-1',
        visible_field_ids: ['field-keep'],
        redacted_field_ids: ['field-redact'],
      },
    ],
    no_originals_left_device: [true],
    ...overrides,
  }
}

function makeCommit(overrides?: Partial<CommitShareOut>): CommitShareOut {
  return {
    kind: 'export_to_person',
    pdf_bytes: PDF_BYTES,
    suggested_filename: 'letter-redacted.pdf',
    output_text: null,
    audit_event_id: 42,
    ...overrides,
  }
}

function mockHappyPreview(preview: SharePreview = makePreview()) {
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'preview_share') return Promise.resolve(preview)
    if (cmd === 'commit_share') return Promise.resolve(makeCommit())
    if (cmd === 'list_variants') return Promise.resolve({ variants: [] })
    if (cmd === 'save_variant') {
      return Promise.resolve({
        variant_id: 'v-new',
        name: 'pack',
        created_at: '2026-08-24T12:00:00Z',
      })
    }
    return Promise.reject(new Error(`unexpected command: ${cmd}`))
  })
}

function baseProps() {
  return {
    docId: 'doc-1',
    sourceFilename: 'letter.txt',
    onLock: vi.fn(),
    onNavigateVault: vi.fn(),
    onNavigateSettings: vi.fn(),
    onNavigateAudit: vi.fn(),
    onDone: vi.fn(),
  }
}

async function loaded() {
  await waitFor(() => {
    expect(screen.getByRole('button', { name: SAVE_REDACTED_PDF_LABEL })).toBeEnabled()
  })
}

beforeEach(() => {
  invokeMock.mockReset()
  saveMock.mockReset()
  writeFileMock.mockReset()
  documentDirMock.mockReset()
  joinMock.mockReset()
  documentDirMock.mockResolvedValue('/Users/me/Documents')
  joinMock.mockImplementation(async (dir: string, name: string) => `${dir}/${name}`)
  // jsdom does not implement blob URLs; the production path uses them for the PDF iframe.
  URL.createObjectURL = vi.fn(() => 'blob:pg-preview')
  URL.revokeObjectURL = vi.fn()
})

describe('ShareScreen — preview (ui.md §10.2, FR-6.1)', () => {
  it('calls preview_share for export_to_person and shows the PDF iframe plus manifest ids', async () => {
    mockHappyPreview()
    render(ShareScreen, baseProps())
    await loaded()

    expect(invokeMock).toHaveBeenCalledWith(
      'preview_share',
      expect.objectContaining({
        input: {
          request: {
            kind: 'export_to_person',
            doc_ids: ['doc-1'],
            per_doc_overrides: {},
            applied_variant_ids: {},
            recipient_note: null,
            ai_instruction: null,
          },
        },
      }),
    )
    const iframe = screen.getByTitle('Redacted PDF preview')
    expect(iframe).toHaveAttribute('src', 'blob:pg-preview')
    expect(screen.getByText('field-keep')).toBeInTheDocument()
    expect(screen.getByText('field-redact')).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: SHARE_TITLE })).toBeInTheDocument()
  })

  it('shows the FR-6.2 warning as a persistent banner when overrides_in_effect, not a toast', async () => {
    mockHappyPreview(makePreview({ overrides_in_effect: true }))
    render(ShareScreen, baseProps())
    await loaded()

    const warning = screen.getByText(EPHEMERAL_OVERRIDE_COPY)
    expect(warning).toBeInTheDocument()
    expect(warning.closest('[role="status"]')).not.toBeNull()
    expect(screen.getByRole('button', { name: SAVE_REDACTED_PDF_LABEL })).toBeEnabled()
  })

  it('does not show the FR-6.2 warning when overrides are not in effect', async () => {
    mockHappyPreview(makePreview({ overrides_in_effect: false }))
    render(ShareScreen, baseProps())
    await loaded()
    expect(screen.queryByText(EPHEMERAL_OVERRIDE_COPY)).not.toBeInTheDocument()
  })
})

describe('ShareScreen — save dialog (ui.md §10.4, C-UI-3)', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('dialog cancel does not call commit_share and stays on preview', async () => {
    mockHappyPreview()
    saveMock.mockResolvedValueOnce(null)
    const props = baseProps()
    render(ShareScreen, props)
    await loaded()

    await fireEvent.click(screen.getByRole('button', { name: SAVE_REDACTED_PDF_LABEL }))

    await waitFor(() => {
      expect(saveMock).toHaveBeenCalled()
    })
    expect(invokeMock).not.toHaveBeenCalledWith('commit_share', expect.anything())
    expect(writeFileMock).not.toHaveBeenCalled()
    expect(props.onDone).not.toHaveBeenCalled()
    expect(screen.getByRole('heading', { name: SHARE_TITLE })).toBeInTheDocument()
    expect(saveMock).toHaveBeenCalledWith(
      expect.objectContaining({
        title: SAVE_REDACTED_PDF_LABEL,
        defaultPath: '/Users/me/Documents/letter-redacted.pdf',
        filters: [{ name: 'PDF', extensions: ['pdf'] }],
      }),
    )
    expect(saveMock.mock.calls[0][0].defaultPath).not.toContain('letter.txt')
  })

  it('confirm path calls commit_share then writes commit pdf_bytes', async () => {
    mockHappyPreview()
    saveMock.mockResolvedValueOnce('/Users/me/Documents/out.pdf')
    writeFileMock.mockResolvedValueOnce(undefined)
    render(ShareScreen, baseProps())
    await loaded()

    await fireEvent.click(screen.getByRole('button', { name: SAVE_REDACTED_PDF_LABEL }))

    await waitFor(() => {
      expect(writeFileMock).toHaveBeenCalled()
    })
    const commitOrder = invokeMock.mock.calls.map((c) => c[0] as string)
    expect(commitOrder.lastIndexOf('commit_share')).toBeGreaterThan(
      commitOrder.lastIndexOf('preview_share'),
    )
    expect(invokeMock).toHaveBeenCalledWith(
      'commit_share',
      expect.objectContaining({ input: { preview_token: 'tok-1' } }),
    )
    const written = writeFileMock.mock.calls[0]
    expect(written[0]).toBe('/Users/me/Documents/out.pdf')
    expect(Array.from(written[1] as Uint8Array)).toEqual(PDF_BYTES)
    await waitFor(() => {
      expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:pg-preview')
    })
    expect(screen.getByRole('status')).toHaveTextContent('Saved out.pdf')
  })

  it('write failure after commit offers Retry save and does not commit a second time', async () => {
    mockHappyPreview()
    saveMock
      .mockResolvedValueOnce('/Users/me/Documents/out.pdf')
      .mockResolvedValueOnce('/Users/me/Documents/retry.pdf')
    writeFileMock.mockRejectedValueOnce(new Error('disk full')).mockResolvedValueOnce(undefined)
    render(ShareScreen, baseProps())
    await loaded()

    await fireEvent.click(screen.getByRole('button', { name: SAVE_REDACTED_PDF_LABEL }))

    expect(await screen.findByText(SHARE_WRITE_FAILED_COPY)).toBeInTheDocument()
    expect(invokeMock.mock.calls.filter((c) => c[0] === 'commit_share')).toHaveLength(1)

    await fireEvent.click(screen.getByRole('button', { name: RETRY_SAVE_LABEL }))
    await waitFor(() => {
      expect(writeFileMock).toHaveBeenCalledTimes(2)
    })
    expect(invokeMock.mock.calls.filter((c) => c[0] === 'commit_share')).toHaveLength(1)
    expect(writeFileMock.mock.calls[1][0]).toBe('/Users/me/Documents/retry.pdf')
  })

  it('surfaces preview_expired on commit and rebuilds the preview without writing', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_variants') return Promise.resolve({ variants: [] })
      if (cmd === 'preview_share') {
        return Promise.resolve(makePreview())
      }
      if (cmd === 'commit_share') {
        return Promise.reject({
          code: 'preview_expired',
          message: 'preview token expired',
        })
      }
      return Promise.reject(new Error(`unexpected command: ${cmd}`))
    })
    saveMock.mockResolvedValue('/Users/me/Documents/out.pdf')
    render(ShareScreen, baseProps())
    await loaded()

    await fireEvent.click(screen.getByRole('button', { name: SAVE_REDACTED_PDF_LABEL }))

    expect(await screen.findByText(PREVIEW_EXPIRED_COPY)).toBeInTheDocument()
    expect(writeFileMock).not.toHaveBeenCalled()
    await waitFor(() => {
      expect(invokeMock.mock.calls.filter((c) => c[0] === 'preview_share').length).toBeGreaterThan(
        1,
      )
    })
  })
})

describe('ShareScreen — Ask Cloud AI (ui.md §10.2–§10.3, §15)', () => {
  it('shows the AI confirm copy and read-only payload before commit', async () => {
    const payload = 'Approved body that will be posted.'
    invokeMock.mockImplementation((cmd: string, args?: { input?: { request?: { kind?: string } } }) => {
      if (cmd === 'list_variants') return Promise.resolve({ variants: [] })
      if (cmd === 'preview_share') {
        if (args?.input?.request?.kind === 'share_to_ai') {
          return Promise.resolve(
            makePreview({
              kind: 'share_to_ai',
              pdf_bytes: null,
              suggested_filename: null,
              ai_payload_preview: payload,
            }),
          )
        }
        return Promise.resolve(makePreview())
      }
      if (cmd === 'commit_share') {
        return Promise.resolve(
          makeCommit({ kind: 'share_to_ai', pdf_bytes: null, output_text: 'Model said hello.' }),
        )
      }
      return Promise.reject(new Error(`unexpected command: ${cmd}`))
    })
    render(ShareScreen, baseProps())
    await loaded()

    await fireEvent.click(screen.getByRole('button', { name: ASK_CLOUD_AI_LABEL }))
    await fireEvent.input(screen.getByLabelText('Instruction'), {
      target: { value: 'Summarize this letter.' },
    })
    await fireEvent.click(screen.getByRole('button', { name: AI_PREVIEW_LABEL }))

    expect(await screen.findByText(AI_CONFIRM_COPY)).toBeInTheDocument()
    expect(screen.getByText(payload)).toBeInTheDocument()
    expect(invokeMock.mock.calls.filter((c) => c[0] === 'commit_share')).toHaveLength(0)

    await fireEvent.click(screen.getByRole('button', { name: SEND_TO_AI_LABEL }))
    expect(await screen.findByText('Model said hello.')).toBeInTheDocument()
    expect(invokeMock).toHaveBeenCalledWith(
      'commit_share',
      expect.objectContaining({ input: { preview_token: 'tok-1' } }),
    )
  })

  it('sends the user to Settings when Cloud AI is not configured', async () => {
    const props = baseProps()
    invokeMock.mockImplementation((cmd: string, args?: { input?: { request?: { kind?: string } } }) => {
      if (cmd === 'list_variants') return Promise.resolve({ variants: [] })
      if (cmd === 'preview_share') {
        if (args?.input?.request?.kind === 'share_to_ai') {
          return Promise.reject({
            code: 'cloud_ai_not_configured',
            message: 'not configured',
          })
        }
        return Promise.resolve(makePreview())
      }
      return Promise.reject(new Error(`unexpected command: ${cmd}`))
    })
    render(ShareScreen, props)
    await fireEvent.click(await screen.findByRole('button', { name: ASK_CLOUD_AI_LABEL }))
    await fireEvent.input(screen.getByLabelText('Instruction'), {
      target: { value: 'Summarize.' },
    })
    await fireEvent.click(screen.getByRole('button', { name: AI_PREVIEW_LABEL }))

    expect(await screen.findByText(CLOUD_AI_NOT_CONFIGURED_COPY)).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: OPEN_SETTINGS_LABEL }))
    expect(props.onNavigateSettings).toHaveBeenCalled()
  })
})

describe('ShareScreen — teardown (ui.md §3.3)', () => {
  it('revokes the blob URL on unmount', async () => {
    mockHappyPreview()
    const { unmount } = render(ShareScreen, baseProps())
    await loaded()
    unmount()
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:pg-preview')
  })
})
