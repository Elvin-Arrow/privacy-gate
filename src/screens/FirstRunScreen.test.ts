import { render, screen, fireEvent, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import FirstRunScreen from './FirstRunScreen.svelte'
import { NO_RECOVERY_COPY } from '../lib/copy'

const invokeMock = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

async function fillAndSubmit(opts: {
  name?: string
  passphrase?: string
  confirm?: string
}) {
  const { name = 'Rosa Delgado', passphrase = 'correct horse battery', confirm = passphrase } =
    opts
  await fireEvent.input(screen.getByLabelText('Display name'), { target: { value: name } })
  await fireEvent.input(screen.getByLabelText('Passphrase'), { target: { value: passphrase } })
  await fireEvent.input(screen.getByLabelText('Confirm passphrase'), {
    target: { value: confirm },
  })
  await fireEvent.click(screen.getByRole('button', { name: 'Create your vault' }))
}

describe('FirstRunScreen', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('shows the C-ARCH-7 non-recovery copy verbatim', () => {
    render(FirstRunScreen, { onSuccess: vi.fn() })
    expect(screen.getByText(NO_RECOVERY_COPY)).toBeInTheDocument()
  })

  it('has autocomplete off on both passphrase fields', () => {
    render(FirstRunScreen, { onSuccess: vi.fn() })
    expect(screen.getByLabelText('Passphrase')).toHaveAttribute('autocomplete', 'off')
    expect(screen.getByLabelText('Confirm passphrase')).toHaveAttribute('autocomplete', 'off')
  })

  it('blocks submit and shows an error on passphrase/confirm mismatch', async () => {
    const onSuccess = vi.fn()
    render(FirstRunScreen, { onSuccess })
    await fillAndSubmit({ passphrase: 'longenoughpass', confirm: 'somethingelse' })

    expect(screen.getByText("Passphrases don't match.")).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalled()
    expect(onSuccess).not.toHaveBeenCalled()
  })

  it('rejects a passphrase shorter than 8 characters before calling invoke', async () => {
    render(FirstRunScreen, { onSuccess: vi.fn() })
    await fillAndSubmit({ passphrase: 'short', confirm: 'short' })

    expect(
      screen.getByText('Passphrase must be at least 8 characters.'),
    ).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('rejects an empty display name before calling invoke', async () => {
    render(FirstRunScreen, { onSuccess: vi.fn() })
    await fillAndSubmit({ name: '   ' })

    expect(screen.getByText('Enter a display name.')).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('surfaces ApiError.message verbatim on account_exists', async () => {
    invokeMock.mockRejectedValueOnce({
      code: 'account_exists',
      message: 'an account already exists',
    })
    render(FirstRunScreen, { onSuccess: vi.fn() })
    await fillAndSubmit({})

    await waitFor(() => {
      expect(screen.getByText('an account already exists')).toBeInTheDocument()
    })
  })

  it('calls create_account and onSuccess on valid submit', async () => {
    invokeMock.mockResolvedValueOnce({ account_id: 'acct-1', state: 'unlocked' })
    const onSuccess = vi.fn()
    render(FirstRunScreen, { onSuccess })
    await fillAndSubmit({})

    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({ account_id: 'acct-1', state: 'unlocked' })
    })
    expect(invokeMock).toHaveBeenCalledWith('create_account', {
      input: { display_name: 'Rosa Delgado', passphrase: 'correct horse battery' },
    })
  })
})
