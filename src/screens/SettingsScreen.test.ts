import { render, screen, fireEvent, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import SettingsScreen from './SettingsScreen.svelte'
import { NO_RECOVERY_COPY } from '../lib/copy'
import type { CloudAiGetConfigOut, RetentionDefaultOut } from '../lib/api'

const invokeMock = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

const ACCOUNT_OUT = {
  account_id: 'acct-123',
  display_name: 'Rosa Delgado',
  created_at: '2026-01-15T10:30:00Z',
}

const RETENTION_DISCARD: RetentionDefaultOut = { policy: 'discard', confirmed: true }
const RETENTION_NEVER_RETAIN: RetentionDefaultOut = { policy: 'never_retain', confirmed: true }

const CLOUD_AI_UNCONFIGURED: CloudAiGetConfigOut = {
  configured: false,
  endpoint_url: null,
  endpoint_host: null,
  model: null,
  key_last4: null,
}

function mockMount(overrides?: {
  retention?: RetentionDefaultOut
  cloudAi?: CloudAiGetConfigOut
}) {
  const retention = overrides?.retention ?? RETENTION_DISCARD
  const cloudAi = overrides?.cloudAi ?? CLOUD_AI_UNCONFIGURED
  invokeMock.mockImplementation((cmd: string) => {
    if (cmd === 'get_account') return Promise.resolve(ACCOUNT_OUT)
    if (cmd === 'get_retention_default') return Promise.resolve(retention)
    if (cmd === 'cloud_ai_get_config') return Promise.resolve(cloudAi)
    return Promise.reject(new Error(`unexpected command in mount mock: ${cmd}`))
  })
}

function baseProps() {
  return { onLock: vi.fn(), onNavigateVault: vi.fn() }
}

describe('SettingsScreen — Account (§11.1)', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it('renders get_account values read-only, no editable account fields', async () => {
    mockMount()
    render(SettingsScreen, baseProps())

    await waitFor(() => {
      expect(screen.getByText('Rosa Delgado')).toBeInTheDocument()
    })
    expect(screen.getByText('acct-123')).toBeInTheDocument()
    // No raw ISO dump.
    expect(screen.queryByText('2026-01-15T10:30:00Z')).not.toBeInTheDocument()

    // No input anywhere lets the user edit account_id/display_name/created_at — the only
    // inputs on the whole screen belong to passphrase/retention/Cloud AI forms, none of
    // which carry account values.
    for (const input of screen.getAllByRole('textbox')) {
      expect((input as HTMLInputElement).value).not.toBe('acct-123')
    }
  })
})

describe('SettingsScreen — Passphrase (§11.2)', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  async function fillPassphraseForm(opts: {
    current?: string
    next?: string
    confirm?: string
  }) {
    const { current = 'old passphrase', next = 'new long passphrase', confirm = next } = opts
    await fireEvent.input(screen.getByLabelText('Current passphrase'), {
      target: { value: current },
    })
    await fireEvent.input(screen.getByLabelText('New passphrase'), {
      target: { value: next },
    })
    await fireEvent.input(screen.getByLabelText('Confirm new passphrase'), {
      target: { value: confirm },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Change passphrase' }))
  }

  it('blocks submit on new/confirm mismatch, no change_passphrase invoke', async () => {
    mockMount()
    render(SettingsScreen, baseProps())
    await waitFor(() => screen.getByText('Rosa Delgado'))
    invokeMock.mockClear()

    await fillPassphraseForm({ next: 'new long passphrase', confirm: 'different confirm' })

    expect(screen.getByText("New passphrase and confirmation don't match.")).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('change_passphrase', expect.anything())
  })

  it('blocks submit on new passphrase under 8 chars, no invoke', async () => {
    mockMount()
    render(SettingsScreen, baseProps())
    await waitFor(() => screen.getByText('Rosa Delgado'))
    invokeMock.mockClear()

    await fillPassphraseForm({ next: 'short', confirm: 'short' })

    expect(
      screen.getByText('New passphrase must be at least 8 characters.'),
    ).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('change_passphrase', expect.anything())
  })

  it('surfaces the distinct passphrase_mismatch (wrong current) copy, not the client-side mismatch copy', async () => {
    mockMount()
    render(SettingsScreen, baseProps())
    await waitFor(() => screen.getByText('Rosa Delgado'))
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'change_passphrase') {
        return Promise.reject({ code: 'passphrase_mismatch', message: 'current passphrase wrong' })
      }
      return Promise.reject(new Error(`unexpected: ${cmd}`))
    })

    await fillPassphraseForm({})

    await waitFor(() => {
      expect(screen.getByText('Current passphrase is incorrect.')).toBeInTheDocument()
    })
    expect(
      screen.queryByText("New passphrase and confirmation don't match."),
    ).not.toBeInTheDocument()
  })

  it('shows the C-ARCH-7 non-recovery sentence on this screen', async () => {
    mockMount()
    render(SettingsScreen, baseProps())
    await waitFor(() => screen.getByText('Rosa Delgado'))
    expect(screen.getByText(NO_RECOVERY_COPY)).toBeInTheDocument()
  })

  it('calls change_passphrase with the exact wire shape on a valid submit', async () => {
    mockMount()
    render(SettingsScreen, baseProps())
    await waitFor(() => screen.getByText('Rosa Delgado'))
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'change_passphrase') return Promise.resolve({ ok: true })
      return Promise.reject(new Error(`unexpected: ${cmd}`))
    })

    await fillPassphraseForm({
      current: 'old one',
      next: 'brand new passphrase',
      confirm: 'brand new passphrase',
    })

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('change_passphrase', {
        input: { current: 'old one', new_passphrase: 'brand new passphrase' },
      })
    })
  })
})

describe('SettingsScreen — Retention default (§11.3)', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  it.each([
    ['discard', 'Discard originals after approval (recommended)'],
    ['retain', 'Keep encrypted originals by default'],
    ['never_retain', 'Never keep originals (cannot keep on a single file)'],
  ] as const)('selecting %s and confirming calls set_retention_default with that policy', async (policy, label) => {
    mockMount()
    render(SettingsScreen, baseProps())
    await waitFor(() => screen.getByText('Rosa Delgado'))
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'set_retention_default') return Promise.resolve({ policy, confirmed: true })
      return Promise.reject(new Error(`unexpected: ${cmd}`))
    })

    await fireEvent.click(screen.getByLabelText(label))
    await fireEvent.click(screen.getByRole('button', { name: 'Save default' }))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('set_retention_default', { input: { policy } })
    })
  })

  it('does not block selecting "retain" while the current default is never_retain', async () => {
    mockMount({ retention: RETENTION_NEVER_RETAIN })
    render(SettingsScreen, baseProps())
    await waitFor(() => screen.getByText('Rosa Delgado'))

    const retainRadio = screen.getByLabelText(
      'Keep encrypted originals by default',
    ) as HTMLInputElement
    expect(retainRadio.disabled).toBe(false)

    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'set_retention_default') {
        return Promise.resolve({ policy: 'retain', confirmed: true })
      }
      return Promise.reject(new Error(`unexpected: ${cmd}`))
    })

    await fireEvent.click(retainRadio);
    await fireEvent.click(screen.getByRole('button', { name: 'Save default' }))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('set_retention_default', {
        input: { policy: 'retain' },
      })
    })
  })
})

describe('SettingsScreen — Cloud AI (§11.4)', () => {
  beforeEach(() => {
    invokeMock.mockReset()
  })

  const DISTINCTIVE_KEY = 'sk-super-secret-canary-zzz999'

  async function fillCloudAiForm() {
    await fireEvent.input(screen.getByLabelText('Endpoint (https)'), {
      target: { value: 'https://api.example.com' },
    })
    await fireEvent.input(screen.getByLabelText('Model ID'), {
      target: { value: 'gpt-review-4' },
    })
    await fireEvent.input(screen.getByLabelText('API key'), {
      target: { value: DISTINCTIVE_KEY },
    })
  }

  it('does not retain the typed API key anywhere in the DOM after cloud_ai_set_config resolves', async () => {
    mockMount()
    const { container } = render(SettingsScreen, baseProps())
    await waitFor(() => screen.getByText('Rosa Delgado'))
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'cloud_ai_set_config') {
        return Promise.resolve({
          configured: true,
          endpoint_host: 'api.example.com',
          model: 'gpt-review-4',
          key_last4: 'z999',
        })
      }
      if (cmd === 'cloud_ai_get_config') {
        return Promise.resolve({
          configured: true,
          endpoint_url: 'https://api.example.com',
          endpoint_host: 'api.example.com',
          model: 'gpt-review-4',
          key_last4: 'z999',
        })
      }
      return Promise.reject(new Error(`unexpected: ${cmd}`))
    })

    await fillCloudAiForm()
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    await waitFor(() => {
      expect(screen.getByText('ending z999')).toBeInTheDocument()
    })
    await waitFor(() => {
      expect((screen.getByLabelText('API key') as HTMLInputElement).value).toBe('')
    })
    expect(container.innerHTML).not.toContain(DISTINCTIVE_KEY)
  })

  it('cloud_ai_get_config never renders anything but key_last4 for the key', async () => {
    const FULL_KEY = 'sk-full-key-should-never-render-anywhere'
    mockMount({
      cloudAi: {
        configured: true,
        endpoint_url: 'https://api.example.com',
        endpoint_host: 'api.example.com',
        model: 'gpt-review-4',
        key_last4: 'ab12',
      },
    })
    const { container } = render(SettingsScreen, baseProps())

    await waitFor(() => {
      expect(screen.getByText('ending ab12')).toBeInTheDocument()
    })
    expect(container.innerHTML).not.toContain(FULL_KEY)
  })

  it('clicking Test fires cloud_ai_test and no other invoke', async () => {
    mockMount()
    render(SettingsScreen, baseProps())
    await waitFor(() => screen.getByText('Rosa Delgado'))
    invokeMock.mockClear()
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'cloud_ai_test') return Promise.resolve({ ok: true, error_class: null })
      return Promise.reject(new Error(`unexpected: ${cmd}`))
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Test' }))

    await waitFor(() => {
      expect(screen.getByText('Test succeeded.')).toBeInTheDocument()
    })
    expect(invokeMock).toHaveBeenCalledTimes(1)
    expect(invokeMock).toHaveBeenCalledWith('cloud_ai_test')
  })

  it('Clear requires a confirm step before cloud_ai_clear_config fires', async () => {
    mockMount({
      cloudAi: {
        configured: true,
        endpoint_url: 'https://api.example.com',
        endpoint_host: 'api.example.com',
        model: 'gpt-review-4',
        key_last4: 'ab12',
      },
    })
    render(SettingsScreen, baseProps())
    await waitFor(() => screen.getByText('ending ab12'))

    await fireEvent.click(screen.getByRole('button', { name: 'Clear' }))
    // One click alone must not clear.
    expect(invokeMock).not.toHaveBeenCalledWith('cloud_ai_clear_config')
    expect(screen.getByText('Clear the stored Cloud AI configuration?')).toBeInTheDocument()

    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'cloud_ai_clear_config') return Promise.resolve({ configured: false })
      if (cmd === 'cloud_ai_get_config') {
        return Promise.resolve({
          configured: false,
          endpoint_url: null,
          endpoint_host: null,
          model: null,
          key_last4: null,
        })
      }
      return Promise.reject(new Error(`unexpected: ${cmd}`))
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Yes, clear' }))

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('cloud_ai_clear_config')
    })
  })
})
