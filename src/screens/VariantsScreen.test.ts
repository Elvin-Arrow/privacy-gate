import { render, screen, fireEvent, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import VariantsScreen from './VariantsScreen.svelte'
import {
  DELETE_VARIANT_LABEL,
  VARIANT_NO_EDIT_COPY,
  VARIANTS_EMPTY_COPY,
  VARIANTS_TITLE,
} from '../lib/copy'

const invokeMock = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

function baseProps() {
  return {
    docId: 'doc-1',
    sourceFilename: 'letter.txt',
    onLock: vi.fn(),
    onNavigateVault: vi.fn(),
    onNavigateSettings: vi.fn(),
    onNavigateAudit: vi.fn(),
  }
}

beforeEach(() => {
  invokeMock.mockReset()
})

describe('VariantsScreen (ui.md §9)', () => {
  it('shows the empty state when list_variants returns none', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_variants') return Promise.resolve({ variants: [] })
      return Promise.reject(new Error(`unexpected command: ${cmd}`))
    })
    render(VariantsScreen, baseProps())

    expect(await screen.findByText(VARIANTS_EMPTY_COPY)).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: VARIANTS_TITLE })).toBeInTheDocument()
    expect(invokeMock).toHaveBeenCalledWith(
      'list_variants',
      expect.objectContaining({ input: { doc_id: 'doc-1' } }),
    )
  })

  it('lists variants and deletes with confirm; no edit control', async () => {
    let variants = [
      { variant_id: 'v1', name: 'Landlord pack', created_at: '2026-08-20T10:00:00Z' },
    ]
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'list_variants') return Promise.resolve({ variants })
      if (cmd === 'delete_variant') {
        variants = []
        return Promise.resolve({ ok: true })
      }
      return Promise.reject(new Error(`unexpected command: ${cmd}`))
    })
    render(VariantsScreen, baseProps())

    expect(await screen.findByText('Landlord pack')).toBeInTheDocument()
    expect(screen.getByText(VARIANT_NO_EDIT_COPY)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /edit/i })).not.toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: DELETE_VARIANT_LABEL }))
    await fireEvent.click(screen.getByRole('button', { name: 'Yes, delete' }))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        'delete_variant',
        expect.objectContaining({ input: { doc_id: 'doc-1', variant_id: 'v1' } }),
      )
    })
    expect(await screen.findByText(VARIANTS_EMPTY_COPY)).toBeInTheDocument()
  })
})
