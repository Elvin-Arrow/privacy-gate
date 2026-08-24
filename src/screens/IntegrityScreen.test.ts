import { render, screen, fireEvent, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import IntegrityScreen from './IntegrityScreen.svelte'
import { INTEGRITY_BODY, INTEGRITY_REPORT_FILENAME, INTEGRITY_TITLE } from '../lib/copy'

const invokeMock = vi.hoisted(() => vi.fn())
const saveMock = vi.hoisted(() => vi.fn())
const writeTextFileMock = vi.hoisted(() => vi.fn())

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: saveMock }))
vi.mock('@tauri-apps/plugin-fs', () => ({ writeTextFile: writeTextFileMock }))

describe('IntegrityScreen', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    saveMock.mockReset()
    writeTextFileMock.mockReset()
  })

  it('shows the §13.1 title and body', () => {
    render(IntegrityScreen, { onLock: vi.fn() })
    expect(screen.getByText(INTEGRITY_TITLE)).toBeInTheDocument()
    expect(screen.getByText(INTEGRITY_BODY)).toBeInTheDocument()
  })

  it('has no "open anyway" / "open documents" control anywhere in the DOM', () => {
    render(IntegrityScreen, { onLock: vi.fn() })
    // Broad, role-based search over every interactive control (C-UI-5): the §13.1 body
    // copy itself legitimately says "cannot open documents" in prose, so the search is
    // scoped to controls' accessible names/text, not raw body text.
    const forbidden = /open anyway|open document/i
    for (const role of ['link', 'button', 'menuitem'] as const) {
      for (const el of screen.queryAllByRole(role)) {
        expect(el).not.toHaveAccessibleName(forbidden)
      }
    }
    for (const el of document.querySelectorAll('a, button')) {
      expect(el.textContent ?? '').not.toMatch(forbidden)
    }
  })

  it('only offers Save report and Lock actions', () => {
    render(IntegrityScreen, { onLock: vi.fn() })
    const buttons = screen.getAllByRole('button').map((b) => b.textContent?.trim())
    expect(buttons).toEqual(['Save report', 'Lock'])
  })

  it('calls onLock when Lock is clicked', async () => {
    const onLock = vi.fn()
    render(IntegrityScreen, { onLock })
    await fireEvent.click(screen.getByRole('button', { name: 'Lock' }))
    expect(onLock).toHaveBeenCalled()
  })

  it('Save report fetches the report, opens the save dialog, and writes it', async () => {
    const report = {
      ok: false,
      kind: 'modification',
      head_sequence: 10,
      tail_sequence: 7,
      first_bad_sequence: 8,
    }
    invokeMock.mockResolvedValueOnce(report)
    saveMock.mockResolvedValueOnce('/home/user/Documents/report.json')
    writeTextFileMock.mockResolvedValueOnce(undefined)

    render(IntegrityScreen, { onLock: vi.fn() })
    await fireEvent.click(screen.getByRole('button', { name: 'Save report' }))

    await waitFor(() => {
      expect(writeTextFileMock).toHaveBeenCalledWith(
        '/home/user/Documents/report.json',
        JSON.stringify(report, null, 2),
      )
    })
    expect(invokeMock).toHaveBeenCalledWith('get_integrity_report')
    expect(saveMock).toHaveBeenCalledWith(
      expect.objectContaining({ defaultPath: INTEGRITY_REPORT_FILENAME }),
    )
  })

  it('does not write when the save dialog is cancelled', async () => {
    invokeMock.mockResolvedValueOnce({
      ok: false,
      kind: 'modification',
      head_sequence: 10,
      tail_sequence: 7,
      first_bad_sequence: 8,
    })
    saveMock.mockResolvedValueOnce(null)

    render(IntegrityScreen, { onLock: vi.fn() })
    await fireEvent.click(screen.getByRole('button', { name: 'Save report' }))

    await waitFor(() => {
      expect(saveMock).toHaveBeenCalled()
    })
    expect(writeTextFileMock).not.toHaveBeenCalled()
  })
})
