import { render, screen, fireEvent, waitFor, within } from '@testing-library/svelte'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import VaultScreen from './VaultScreen.svelte'
import type { DocumentSummary, RetentionDefaultOut } from '../lib/api'
import {
  IMPORT_INVALID_INPUT_COPY,
  OVER_BUDGET_COPY,
  RETENTION_LOOSEN_FORBIDDEN_COPY,
  UNSUPPORTED_DOCUMENT_COPY,
  RETENTION_POLICY_UNSET_COPY,
} from '../lib/copy'

const invokeMock = vi.hoisted(() => vi.fn())
const listenMock = vi.hoisted(() => vi.fn())

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))

// dev-plan W32 explicitly-named test: "no plugin-fs read" — assert this module tree never
// even imports the plugin so a read-oriented call is structurally impossible, not merely
// unobserved in a given test run.
vi.mock('@tauri-apps/plugin-fs', () => {
  throw new Error('plugin-fs must never be imported by the import flow (C-UI-1 / §12)')
})

const RETENTION_UNCONFIRMED: RetentionDefaultOut = { policy: 'discard', confirmed: false }
const RETENTION_CONFIRMED: RetentionDefaultOut = { policy: 'discard', confirmed: true }
const RETENTION_NEVER_RETAIN_CONFIRMED: RetentionDefaultOut = {
  policy: 'never_retain',
  confirmed: true,
}

function docSummary(overrides?: Partial<DocumentSummary>): DocumentSummary {
  return {
    doc_id: 'doc-1',
    source_filename: 'report.pdf',
    source_format: 'pdf',
    imported_at: '2026-08-24T10:00:00Z',
    retention: 'discard',
    has_approved_version: false,
    has_retained_original: false,
    detected_field_count: 3,
    ...overrides,
  }
}

function mockMount(options?: {
  retention?: RetentionDefaultOut
  documents?: DocumentSummary[]
}) {
  const retention = options?.retention ?? RETENTION_CONFIRMED
  let documents = options?.documents ?? []
  invokeMock.mockImplementation((cmd: string, args?: unknown) => {
    if (cmd === 'get_retention_default') return Promise.resolve(retention)
    if (cmd === 'list_documents') return Promise.resolve({ documents })
    if (cmd === 'set_retention_default') {
      const input = (args as { input: { policy: string } }).input
      return Promise.resolve({ policy: input.policy, confirmed: true })
    }
    if (cmd === 'delete_document') {
      const input = (args as { input: { doc_id: string } }).input
      documents = documents.filter((d) => d.doc_id !== input.doc_id)
      return Promise.resolve({ ok: true })
    }
    return Promise.reject(new Error(`unexpected command in mount mock: ${cmd}`))
  })
  return {
    setDocuments(next: DocumentSummary[]) {
      documents = next
    },
  }
}

function baseProps() {
  return { onLock: vi.fn(), onNavigateSettings: vi.fn() }
}

function makeFile(name: string, content = 'hello', type = 'text/plain'): File {
  return new File([content], name, { type })
}

beforeEach(() => {
  invokeMock.mockReset()
  listenMock.mockReset()
  listenMock.mockResolvedValue(() => {})
})

describe('VaultScreen — retention gate (§6, decision 0007)', () => {
  it('shows the blocking modal when confirmed is false', async () => {
    mockMount({ retention: RETENTION_UNCONFIRMED })
    render(VaultScreen, baseProps())

    await waitFor(() => screen.getByText('Import a document'))
    await fireEvent.click(screen.getByRole('button', { name: 'Import a document' }))

    expect(
      await screen.findByRole('heading', { name: 'Choose a default for original files' }),
    ).toBeInTheDocument()
  })

  it('does not show the modal when confirmed is already true', async () => {
    mockMount({ retention: RETENTION_CONFIRMED })
    render(VaultScreen, baseProps())

    await waitFor(() => screen.getByText('Import a document'))
    await fireEvent.click(screen.getByRole('button', { name: 'Import a document' }))

    expect(
      screen.queryByRole('heading', { name: 'Choose a default for original files' }),
    ).not.toBeInTheDocument()
  })

  it('pre-selects Discard in the modal', async () => {
    mockMount({ retention: RETENTION_UNCONFIRMED })
    render(VaultScreen, baseProps())

    await fireEvent.click(await screen.findByRole('button', { name: 'Import a document' }))
    const discardRadio = await screen.findByRole('radio', {
      name: /Discard originals after approval/,
    })
    expect(discardRadio).toBeChecked()
  })

  it('Continue calls set_retention_default before import_document (call order, not just both happening)', async () => {
    mockMount({ retention: RETENTION_UNCONFIRMED })
    invokeMock.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === 'get_retention_default') return Promise.resolve(RETENTION_UNCONFIRMED)
      if (cmd === 'list_documents') return Promise.resolve({ documents: [] })
      if (cmd === 'set_retention_default') {
        const input = (args as { input: { policy: string } }).input
        return Promise.resolve({ policy: input.policy, confirmed: true })
      }
      if (cmd === 'import_document') {
        return Promise.resolve({ summary: docSummary(), over_budget: false })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    const { container } = render(VaultScreen, baseProps())

    await fireEvent.click(await screen.findByRole('button', { name: 'Import a document' }))
    await fireEvent.click(await screen.findByRole('button', { name: 'Continue' }))

    // §6 step 4: Continue opens the picker; simulate the user then choosing a file.
    const input = container.querySelector('input[type="file"]') as HTMLInputElement
    await fireEvent.change(input, { target: { files: [makeFile('a.txt')] } })

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('import_document', expect.anything())
    })

    const calledCommands = invokeMock.mock.calls.map((call) => call[0])
    const setIndex = calledCommands.indexOf('set_retention_default')
    const importIndex = calledCommands.indexOf('import_document')
    expect(setIndex).toBeGreaterThanOrEqual(0)
    expect(importIndex).toBeGreaterThan(setIndex)
  })

  it('Cancel does not call import_document and does not proceed to import', async () => {
    mockMount({ retention: RETENTION_UNCONFIRMED })
    render(VaultScreen, baseProps())

    await fireEvent.click(await screen.findByRole('button', { name: 'Import a document' }))
    await fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }))

    expect(
      screen.queryByRole('heading', { name: 'Choose a default for original files' }),
    ).not.toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('import_document', expect.anything())
    expect(invokeMock).not.toHaveBeenCalledWith('set_retention_default', expect.anything())
  })
})

describe('VaultScreen — import write path (§7.2)', () => {
  it('selecting a file calls import_document with { filename, bytes, retention_override } shaped correctly', async () => {
    mockMount({ retention: RETENTION_CONFIRMED })
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_retention_default') return Promise.resolve(RETENTION_CONFIRMED)
      if (cmd === 'list_documents') return Promise.resolve({ documents: [] })
      if (cmd === 'import_document') {
        return Promise.resolve({ summary: docSummary(), over_budget: false })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    const { container } = render(VaultScreen, baseProps())
    await waitFor(() => screen.getByText('Import a document'))

    const input = container.querySelector('input[type="file"]') as HTMLInputElement
    const file = makeFile('report.txt', 'hello world')
    await fireEvent.change(input, { target: { files: [file] } })

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        'import_document',
        expect.objectContaining({
          input: expect.objectContaining({
            filename: 'report.txt',
            retention_override: null,
          }),
        }),
      )
    })
    const call = invokeMock.mock.calls.find((c) => c[0] === 'import_document')
    expect(Array.isArray(call?.[1].input.bytes)).toBe(true)
    expect(call?.[1].input.bytes.length).toBeGreaterThan(0)
  })

  it('rejects an adversarial File.name with path separators before calling import_document', async () => {
    mockMount({ retention: RETENTION_CONFIRMED })
    const { container } = render(VaultScreen, baseProps())
    await waitFor(() => screen.getByText('Import a document'))

    const input = container.querySelector('input[type="file"]') as HTMLInputElement
    const file = makeFile('../../etc/evil.txt', 'x')
    await fireEvent.change(input, { target: { files: [file] } })

    await waitFor(() => {
      expect(screen.getByText(IMPORT_INVALID_INPUT_COPY)).toBeInTheDocument()
    })
    expect(invokeMock).not.toHaveBeenCalledWith('import_document', expect.anything())
  })

  it('never imports @tauri-apps/plugin-fs read functions anywhere in this module', async () => {
    // The top-level vi.mock above throws if the module is ever imported; simply mounting
    // and running the full import flow without that throw firing proves it.
    mockMount({ retention: RETENTION_CONFIRMED })
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_retention_default') return Promise.resolve(RETENTION_CONFIRMED)
      if (cmd === 'list_documents') return Promise.resolve({ documents: [] })
      if (cmd === 'import_document') {
        return Promise.resolve({ summary: docSummary(), over_budget: false })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    const { container } = render(VaultScreen, baseProps())
    await waitFor(() => screen.getByText('Import a document'))
    const input = container.querySelector('input[type="file"]') as HTMLInputElement
    await fireEvent.change(input, { target: { files: [makeFile('a.txt')] } })
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('import_document', expect.anything())
    })
  })

  it('progress bar reflects pg://detect-progress fraction updates', async () => {
    mockMount({ retention: RETENTION_CONFIRMED })
    let progressHandler: ((event: { payload: { fraction: number } }) => void) | undefined
    listenMock.mockImplementation((_name: string, handler: typeof progressHandler) => {
      progressHandler = handler
      return Promise.resolve(() => {})
    })
    let resolveImport: (value: unknown) => void = () => {}
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_retention_default') return Promise.resolve(RETENTION_CONFIRMED)
      if (cmd === 'list_documents') return Promise.resolve({ documents: [] })
      if (cmd === 'import_document') {
        return new Promise((resolve) => {
          resolveImport = resolve
        })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    const { container } = render(VaultScreen, baseProps())
    await waitFor(() => screen.getByText('Import a document'))
    const input = container.querySelector('input[type="file"]') as HTMLInputElement
    await fireEvent.change(input, { target: { files: [makeFile('a.txt')] } })

    await waitFor(() => expect(progressHandler).toBeDefined())
    progressHandler?.({ payload: { fraction: 0.5 } })
    await waitFor(() => {
      expect(screen.getByText('50%')).toBeInTheDocument()
    })

    resolveImport({ summary: docSummary(), over_budget: false })
  })

  it('over_budget shows §15 copy and the document is not discarded', async () => {
    mockMount({ retention: RETENTION_CONFIRMED })
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_retention_default') return Promise.resolve(RETENTION_CONFIRMED)
      if (cmd === 'list_documents') return Promise.resolve({ documents: [docSummary()] })
      if (cmd === 'import_document') {
        return Promise.resolve({ summary: docSummary(), over_budget: true })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    const { container } = render(VaultScreen, baseProps())
    await waitFor(() => screen.getByText('Import a document'))
    const input = container.querySelector('input[type="file"]') as HTMLInputElement
    await fireEvent.change(input, { target: { files: [makeFile('a.txt')] } })

    await waitFor(() => {
      expect(screen.getByText(OVER_BUDGET_COPY)).toBeInTheDocument()
    })
    expect(screen.getByText('report.pdf')).toBeInTheDocument()
  })

  it.each([
    ['unsupported_document', UNSUPPORTED_DOCUMENT_COPY],
    ['retention_policy_unset', RETENTION_POLICY_UNSET_COPY],
    ['retention_loosen_forbidden', RETENTION_LOOSEN_FORBIDDEN_COPY],
    ['invalid_input', IMPORT_INVALID_INPUT_COPY],
  ])('maps %s to comprehensible copy without silently failing', async (code, expectedCopy) => {
    mockMount({ retention: RETENTION_CONFIRMED })
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_retention_default') return Promise.resolve(RETENTION_CONFIRMED)
      if (cmd === 'list_documents') return Promise.resolve({ documents: [] })
      if (cmd === 'import_document') {
        return Promise.reject({ code, message: 'server said no' })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    const { container } = render(VaultScreen, baseProps())
    await waitFor(() => screen.getByText('Import a document'))
    const input = container.querySelector('input[type="file"]') as HTMLInputElement
    await fireEvent.change(input, { target: { files: [makeFile('a.txt')] } })

    await waitFor(() => {
      expect(screen.getByText(expectedCopy)).toBeInTheDocument()
    })
  })
})

describe('VaultScreen — vault list (§7.1)', () => {
  it('shows the import prompt with no fake rows when empty', async () => {
    mockMount({ retention: RETENTION_CONFIRMED, documents: [] })
    render(VaultScreen, baseProps())

    await waitFor(() => {
      expect(screen.getByText(/No documents yet/)).toBeInTheDocument()
    })
    expect(screen.queryByRole('table')).not.toBeInTheDocument()
  })

  it('populated rows show documented columns, newest first, no span text', async () => {
    const docs = [
      docSummary({ doc_id: 'doc-a', source_filename: 'alpha.txt', detected_field_count: 5 }),
      docSummary({ doc_id: 'doc-b', source_filename: 'beta.pdf', detected_field_count: 2 }),
    ]
    mockMount({ retention: RETENTION_CONFIRMED, documents: docs })
    render(VaultScreen, baseProps())

    const table = await screen.findByRole('table')
    const rows = within(table).getAllByRole('row')
    // header + 2 data rows; API already returns newest-first, so DOM order must match.
    expect(rows).toHaveLength(3)
    expect(within(rows[1]).getByText('alpha.txt')).toBeInTheDocument()
    expect(within(rows[1]).getByText('5')).toBeInTheDocument()
    expect(within(rows[2]).getByText('beta.pdf')).toBeInTheDocument()
    // No span text / field labels — only a bare count.
    expect(screen.queryByText(/John Smith|SSN|email address/i)).not.toBeInTheDocument()
  })

  it('refreshes the list after a successful import', async () => {
    let listCallCount = 0
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_retention_default') return Promise.resolve(RETENTION_CONFIRMED)
      if (cmd === 'list_documents') {
        listCallCount += 1
        return Promise.resolve({
          documents: listCallCount > 1 ? [docSummary()] : [],
        })
      }
      if (cmd === 'import_document') {
        return Promise.resolve({ summary: docSummary(), over_budget: false })
      }
      throw new Error(`unexpected command: ${cmd}`)
    })
    const { container } = render(VaultScreen, baseProps())
    await waitFor(() => screen.getByText(/No documents yet/))

    const input = container.querySelector('input[type="file"]') as HTMLInputElement
    await fireEvent.change(input, { target: { files: [makeFile('a.txt')] } })

    await waitFor(() => {
      expect(listCallCount).toBeGreaterThan(1)
    })
    expect(await screen.findByText('report.pdf')).toBeInTheDocument()
  })

  it('re-importing the same filename shows no duplicate warning', async () => {
    mockMount({
      retention: RETENTION_CONFIRMED,
      documents: [docSummary({ source_filename: 'same.txt' })],
    })
    render(VaultScreen, baseProps())

    await screen.findByText('same.txt')
    expect(screen.queryByText(/already imported/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/duplicate/i)).not.toBeInTheDocument()
  })
})

describe('VaultScreen — Delete row action', () => {
  it('requires confirm before delete_document fires', async () => {
    mockMount({
      retention: RETENTION_CONFIRMED,
      documents: [docSummary({ doc_id: 'doc-x', source_filename: 'x.txt' })],
    })
    render(VaultScreen, baseProps())

    await screen.findByText('x.txt')
    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }))

    expect(invokeMock).not.toHaveBeenCalledWith('delete_document', expect.anything())
    expect(screen.getByText(/cannot be undone/)).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Yes, delete' }))
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        'delete_document',
        expect.objectContaining({ input: { doc_id: 'doc-x' } }),
      )
    })
  })
})
