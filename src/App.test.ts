import { render, screen, fireEvent, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import App from './App.svelte'

const invokeMock = vi.hoisted(() => vi.fn())
const listenMock = vi.hoisted(() => vi.fn())

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))

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

  it('lock from the Vault placeholder returns to Unlock and sets the "— Locked" title', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'get_session_state') return Promise.resolve({ state: 'unlocked' })
      if (cmd === 'lock') return Promise.resolve({ state: 'locked' })
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
})
