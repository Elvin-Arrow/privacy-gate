import { render, screen, fireEvent, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import App from './App.svelte'

const invokeMock = vi.hoisted(() => vi.fn())
const listenMock = vi.hoisted(() => vi.fn())

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }))
vi.mock('@tauri-apps/plugin-fs', () => ({ writeFile: vi.fn(), writeTextFile: vi.fn() }))
vi.mock('@tauri-apps/api/path', () => ({
  documentDir: vi.fn(async () => '/Users/me/Documents'),
  join: vi.fn(async (dir: string, name: string) => `${dir}/${name}`),
}))

describe('App routing', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    listenMock.mockReset()
    listenMock.mockResolvedValue(() => {})
    document.title = ''
  })

  it('renders the first-run chrome synchronously off get_session_state alone (§14 proxy)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') {
        return Promise.resolve({ state: 'first_run' })
      }
      throw new Error(`unexpected command in first-paint test: ${cmd}`)
    })
    render(App)

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Create your vault' })).toBeInTheDocument()
    })
    // No command beyond get_session_state was needed to reach that chrome.
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('get_session_state')
  })

  it('renders the locked chrome synchronously off get_session_state alone (§14 proxy)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') {
        return Promise.resolve({ state: 'locked' })
      }
      throw new Error(`unexpected command in first-paint test: ${cmd}`)
    })
    render(App)

    await waitFor(() => {
      expect(screen.getByText('Unlock your vault')).toBeInTheDocument()
    })
    expect(invokeMock).toHaveBeenCalledTimes(1)
  })

  it('navigates to the Vault placeholder on unlock success', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') return Promise.resolve({ state: 'locked' })
      if (cmd === 'unlock') return Promise.resolve({ state: 'unlocked', integrity: null })
      if (cmd === 'list_documents') return Promise.resolve({ documents: [] })
      if (cmd === 'get_retention_default') {
        return Promise.resolve({ policy: 'discard', confirmed: true })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(App)

    await waitFor(() => screen.getByLabelText('Passphrase'))
    await fireEvent.input(screen.getByLabelText('Passphrase'), {
      target: { value: 'correct horse battery' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Unlock' }))

    await waitFor(() => {
      expect(screen.getByText(/No documents yet/)).toBeInTheDocument()
    })
  })

  it('navigates to the Integrity screen (never Vault) on degraded_integrity', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') return Promise.resolve({ state: 'locked' })
      if (cmd === 'unlock') {
        return Promise.resolve({
          state: 'degraded_integrity',
          integrity: {
            ok: false,
            kind: 'modification',
            head_sequence: 10,
            tail_sequence: 7,
            first_bad_sequence: 8,
          },
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(App)

    await waitFor(() => screen.getByLabelText('Passphrase'))
    await fireEvent.input(screen.getByLabelText('Passphrase'), {
      target: { value: 'correct horse battery' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Unlock' }))

    await waitFor(() => {
      expect(screen.getByText('This vault cannot open documents')).toBeInTheDocument()
    })
    // The explicitly named ui.md §16 test: no Vault-only content reachable from here.
    expect(screen.queryByText(/No documents yet/)).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Lock' })).toBeInTheDocument()
    expect(screen.queryByText('Vault')).not.toBeInTheDocument()
  })

  it('pg://session-changed to degraded_integrity leaves Vault and shows Integrity (W35)', async () => {
    let sessionListener: ((event: { payload: { state: string } }) => void) | undefined
    listenMock.mockImplementation((_name: string, cb: (event: { payload: { state: string } }) => void) => {
      sessionListener = cb
      return Promise.resolve(() => {})
    })
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') return Promise.resolve({ state: 'unlocked' })
      if (cmd === 'list_documents') return Promise.resolve({ documents: [] })
      if (cmd === 'get_retention_default') {
        return Promise.resolve({ policy: 'discard', confirmed: true })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(App)

    await waitFor(() => {
      expect(screen.getByText(/No documents yet/)).toBeInTheDocument()
    })
    await waitFor(() => {
      expect(sessionListener).toBeDefined()
    })
    sessionListener!({ payload: { state: 'degraded_integrity' } })

    await waitFor(() => {
      expect(screen.getByText('This vault cannot open documents')).toBeInTheDocument()
    })
    expect(screen.queryByText(/No documents yet/)).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Vault' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Open anyway' })).not.toBeInTheDocument()
  })

  it('Audit trail nav from the unlocked chrome lands on AuditScreen (W35)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') return Promise.resolve({ state: 'unlocked' })
      if (cmd === 'list_documents') return Promise.resolve({ documents: [] })
      if (cmd === 'get_retention_default') {
        return Promise.resolve({ policy: 'discard', confirmed: true })
      }
      if (cmd === 'list_audit_events') return Promise.resolve({ events: [], next_sequence: null })
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(App)

    await waitFor(() => screen.getByText(/No documents yet/))
    await fireEvent.click(screen.getByRole('button', { name: 'Audit trail' }))

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Audit trail' })).toBeInTheDocument()
    })
    expect(screen.getByText('No audit events match this filter.')).toBeInTheDocument()
  })

  it('lock from the Vault placeholder returns to Unlock and sets the "— Locked" title', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') return Promise.resolve({ state: 'unlocked' })
      if (cmd === 'lock') return Promise.resolve({ state: 'locked' })
      if (cmd === 'list_documents') return Promise.resolve({ documents: [] })
      if (cmd === 'get_retention_default') {
        return Promise.resolve({ policy: 'discard', confirmed: true })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(App)

    await waitFor(() => screen.getByText(/No documents yet/))
    expect(document.title).toBe('Privacy Gate')

    await fireEvent.click(screen.getByRole('button', { name: 'Lock' }))

    await waitFor(() => {
      expect(screen.getByText('Unlock your vault')).toBeInTheDocument()
    })
    expect(document.title).toBe('Privacy Gate — Locked')
  })

  it('Settings nav from the unlocked chrome lands on SettingsScreen (W31)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') return Promise.resolve({ state: 'unlocked' })
      if (cmd === 'list_documents') return Promise.resolve({ documents: [] })
      if (cmd === 'get_account') {
        return Promise.resolve({
          account_id: 'acct-1',
          display_name: 'Rosa Delgado',
          created_at: '2026-01-15T10:30:00Z',
        })
      }
      if (cmd === 'get_retention_default') {
        return Promise.resolve({ policy: 'discard', confirmed: true })
      }
      if (cmd === 'cloud_ai_get_config') {
        return Promise.resolve({
          configured: false,
          endpoint_url: null,
          endpoint_host: null,
          model: null,
          key_last4: null,
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(App)

    await waitFor(() => screen.getByText(/No documents yet/))

    await fireEvent.click(screen.getByRole('button', { name: 'Settings' }))

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Account' })).toBeInTheDocument()
    })
    expect(screen.getByText('Rosa Delgado')).toBeInTheDocument()
  })

  it('Open on an unapproved vault row lands on the approval screen (W33)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') return Promise.resolve({ state: 'unlocked' })
      if (cmd === 'list_documents') {
        return Promise.resolve({
          documents: [
            {
              doc_id: 'doc-1',
              source_filename: 'letter.txt',
              source_format: 'text',
              imported_at: '2026-08-24T10:00:00Z',
              retention: 'discard',
              has_approved_version: false,
              has_retained_original: false,
              detected_field_count: 1,
            },
          ],
        })
      }
      if (cmd === 'get_retention_default') {
        return Promise.resolve({ policy: 'discard', confirmed: true })
      }
      if (cmd === 'open_approval') {
        return Promise.resolve({
          approval_session_id: 'sess-1',
          doc_id: 'doc-1',
          lifecycle: 'awaiting_decisions',
          pages: [{ page_index: 0, spans: [{ byte_offset: 0, text: 'Hello Rosa', page_index: 0 }] }],
          fields: [
            {
              id: 'f1',
              label: 'Name',
              classification: 'person_name',
              span: { byte_offset: 6, byte_length: 4, text: 'Rosa', page_index: 0 },
              parent_field_id: null,
            },
          ],
        })
      }
      if (cmd === 'abort_approval') return Promise.resolve({ lifecycle: 'aborted' })
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(App)

    await waitFor(() => screen.getByText('letter.txt'))
    await fireEvent.click(screen.getByRole('button', { name: 'Open' }))

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Review before approving' })).toBeInTheDocument()
    })
    expect(screen.getByRole('button', { name: 'Approve and store' })).toBeDisabled()
  })

  it('Open on an approved vault row lands on the share preview (W34)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') return Promise.resolve({ state: 'unlocked' })
      if (cmd === 'list_documents') {
        return Promise.resolve({
          documents: [
            {
              doc_id: 'doc-1',
              source_filename: 'letter.txt',
              source_format: 'text',
              imported_at: '2026-08-24T10:00:00Z',
              retention: 'discard',
              has_approved_version: true,
              has_retained_original: false,
              detected_field_count: 1,
            },
          ],
        })
      }
      if (cmd === 'get_retention_default') {
        return Promise.resolve({ policy: 'discard', confirmed: true })
      }
      if (cmd === 'list_variants') return Promise.resolve({ variants: [] })
      if (cmd === 'preview_share') {
        return Promise.resolve({
          preview_token: 'tok-1',
          expires_at: '2026-08-24T12:00:00Z',
          kind: 'export_to_person',
          overrides_in_effect: false,
          suggested_filename: 'letter-redacted.pdf',
          pdf_bytes: [37, 80, 68, 70],
          ai_payload_preview: null,
          manifest: [{ doc_id: 'doc-1', visible_field_ids: [], redacted_field_ids: ['f1'] }],
          no_originals_left_device: [true],
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    URL.createObjectURL = vi.fn(() => 'blob:pg-preview')
    URL.revokeObjectURL = vi.fn()
    render(App)

    await waitFor(() => screen.getByText('letter.txt'))
    await fireEvent.click(screen.getByRole('button', { name: 'Open' }))

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Share preview' })).toBeInTheDocument()
    })
    expect(screen.getByRole('button', { name: 'Save redacted PDF' })).toBeEnabled()
  })

  it('Manage variants on an approved row lands on the variants empty state (W36)', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') return Promise.resolve({ state: 'unlocked' })
      if (cmd === 'list_documents') {
        return Promise.resolve({
          documents: [
            {
              doc_id: 'doc-1',
              source_filename: 'letter.txt',
              source_format: 'text',
              imported_at: '2026-08-24T10:00:00Z',
              retention: 'discard',
              has_approved_version: true,
              has_retained_original: false,
              detected_field_count: 1,
            },
          ],
        })
      }
      if (cmd === 'get_retention_default') {
        return Promise.resolve({ policy: 'discard', confirmed: true })
      }
      if (cmd === 'list_variants') return Promise.resolve({ variants: [] })
      throw new Error(`unexpected command: ${cmd}`)
    })
    render(App)

    await waitFor(() => screen.getByText('letter.txt'))
    await fireEvent.click(screen.getByRole('button', { name: 'Manage variants' }))

    await waitFor(() => {
      expect(screen.getByRole('heading', { name: 'Variants' })).toBeInTheDocument()
    })
    expect(
      screen.getByText(/No saved variants for this document/),
    ).toBeInTheDocument()
  })
})
