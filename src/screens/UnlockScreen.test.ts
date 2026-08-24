import { render, screen, fireEvent, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import UnlockScreen from './UnlockScreen.svelte'
import { NO_RECOVERY_COPY, UNLOCK_FAILED_COPY } from '../lib/copy'

const invokeMock = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

describe('UnlockScreen', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('has no "forgot passphrase" / reset / recover control anywhere in the DOM', () => {
    render(UnlockScreen, { onUnlocked: vi.fn() })

    // Broad, role-based search (not just "we didn't add a button named X"): every
    // clickable/navigable control on the screen, by accessible name. The C-ARCH-7
    // non-recovery sentence itself legitimately uses "reset"/"recovery" in prose, so the
    // search is scoped to interactive controls, not raw body text.
    const forbidden = /forgot|reset|recover/i
    const interactiveRoles = ['link', 'button', 'menuitem'] as const
    for (const role of interactiveRoles) {
      for (const el of screen.queryAllByRole(role)) {
        expect(el).not.toHaveAccessibleName(forbidden)
      }
    }
    // Also no bare <a>/<button> element whose own text matches, independent of role query.
    for (const el of document.querySelectorAll('a, button')) {
      expect(el.textContent ?? '').not.toMatch(forbidden)
    }
  })

  it('has autocomplete off on the passphrase field', () => {
    render(UnlockScreen, { onUnlocked: vi.fn() })
    expect(screen.getByLabelText('Passphrase')).toHaveAttribute('autocomplete', 'off')
  })

  it('shows the exact unlock_failed copy on a wrong passphrase', async () => {
    invokeMock.mockRejectedValueOnce({ code: 'unlock_failed', message: 'unlock failed' })
    render(UnlockScreen, { onUnlocked: vi.fn() })

    await fireEvent.input(screen.getByLabelText('Passphrase'), {
      target: { value: 'wrong-pass' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Unlock' }))

    await waitFor(() => {
      expect(screen.getByText(UNLOCK_FAILED_COPY)).toBeInTheDocument()
    })
  })

  it('shows the C-ARCH-7 non-recovery copy', () => {
    render(UnlockScreen, { onUnlocked: vi.fn() })
    expect(screen.getByText(NO_RECOVERY_COPY)).toBeInTheDocument()
  })

  it('calls onUnlocked with state "unlocked" on success', async () => {
    invokeMock.mockResolvedValueOnce({ state: 'unlocked', integrity: null })
    const onUnlocked = vi.fn()
    render(UnlockScreen, { onUnlocked })

    await fireEvent.input(screen.getByLabelText('Passphrase'), {
      target: { value: 'correct horse battery' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Unlock' }))

    await waitFor(() => {
      expect(onUnlocked).toHaveBeenCalledWith({ state: 'unlocked', integrity: null })
    })
    expect(invokeMock).toHaveBeenCalledWith('unlock', {
      input: { passphrase: 'correct horse battery' },
    })
  })

  it('calls onUnlocked with state "degraded_integrity" and the report', async () => {
    const integrity = {
      ok: false,
      kind: 'modification',
      head_sequence: 10,
      tail_sequence: 7,
      first_bad_sequence: 8,
    }
    invokeMock.mockResolvedValueOnce({ state: 'degraded_integrity', integrity })
    const onUnlocked = vi.fn()
    render(UnlockScreen, { onUnlocked })

    await fireEvent.input(screen.getByLabelText('Passphrase'), {
      target: { value: 'correct horse battery' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Unlock' }))

    await waitFor(() => {
      expect(onUnlocked).toHaveBeenCalledWith({ state: 'degraded_integrity', integrity })
    })
  })
})
