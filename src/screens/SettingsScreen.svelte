<script lang="ts">
  // ui.md §11 — one Settings screen, four subsections, no invented settings (dev-plan W31
  // "Do not: invent extra settings"). Kept as one file (not split under screens/settings/)
  // since each subsection is a handful of fields/one form — splitting would mean four tiny
  // files sharing one mount-time fetch pattern for no real separation-of-concerns win,
  // unlike e.g. Approval's two-pane layout in a later chunk.

  import { onMount } from 'svelte'
  import AppShell from '../lib/AppShell.svelte'
  import {
    changePassphrase,
    cloudAiClearConfig,
    cloudAiGetConfig,
    cloudAiSetConfig,
    cloudAiTest,
    getAccount,
    getRetentionDefault,
    isApiError,
    setRetentionDefault,
    type RetentionPolicy,
  } from '../lib/api'
  import {
    CLOUD_AI_SCOPE_COPY,
    NO_RECOVERY_COPY,
    PASSPHRASE_CONFIRM_MISMATCH_COPY,
    PASSPHRASE_CURRENT_WRONG_COPY,
    RETENTION_POLICY_LABELS,
  } from '../lib/copy'

  const MIN_PASSPHRASE_LENGTH = 8
  const RETENTION_POLICIES: RetentionPolicy[] = ['discard', 'retain', 'never_retain']

  let { onLock, onNavigateVault }: { onLock: () => void; onNavigateVault: () => void } = $props()

  // --- Account (§11.1) --------------------------------------------------
  let accountId = $state('')
  let displayName = $state('')
  let createdAt = $state('')

  const createdAtFormatted = $derived(formatDate(createdAt))

  function formatDate(iso: string): string {
    if (iso === '') return ''
    const parsed = new Date(iso)
    if (Number.isNaN(parsed.getTime())) return iso
    return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(
      parsed,
    )
  }

  // --- Passphrase (§11.2) ------------------------------------------------
  let currentPassphrase = $state('')
  let newPassphrase = $state('')
  let confirmNewPassphrase = $state('')
  let passphraseSubmitting = $state(false)
  let passphraseError = $state('')
  let passphraseSuccess = $state('')

  function validatePassphrase(): boolean {
    passphraseError = ''
    if (newPassphrase.length < MIN_PASSPHRASE_LENGTH) {
      passphraseError = `New passphrase must be at least ${MIN_PASSPHRASE_LENGTH} characters.`
      return false
    }
    if (newPassphrase !== confirmNewPassphrase) {
      passphraseError = PASSPHRASE_CONFIRM_MISMATCH_COPY
      return false
    }
    return true
  }

  async function handleChangePassphrase(event: SubmitEvent) {
    event.preventDefault()
    passphraseSuccess = ''
    if (!validatePassphrase()) {
      return
    }
    passphraseSubmitting = true
    try {
      await changePassphrase({ current: currentPassphrase, new_passphrase: newPassphrase })
      passphraseSuccess = 'Passphrase changed.'
      currentPassphrase = ''
      newPassphrase = ''
      confirmNewPassphrase = ''
    } catch (err) {
      // api.md §3: `passphrase_mismatch` means the *current* passphrase was wrong — a
      // server-side rejection, distinct from the client-side new/confirm mismatch check
      // above. Never show the same copy for both (see dev-log 0043 "Ambiguities").
      passphraseError =
        isApiError(err) && err.code === 'passphrase_mismatch'
          ? PASSPHRASE_CURRENT_WRONG_COPY
          : isApiError(err)
            ? err.message
            : 'Could not change the passphrase.'
    } finally {
      passphraseSubmitting = false
    }
  }

  // --- Retention default (§11.3) -----------------------------------------
  let retentionPolicy = $state<RetentionPolicy>('discard')
  let retentionSaving = $state(false)
  let retentionStatus = $state('')

  async function handleSaveRetention() {
    retentionStatus = ''
    retentionSaving = true
    try {
      const out = await setRetentionDefault(retentionPolicy)
      retentionPolicy = out.policy
      retentionStatus = 'Default saved.'
    } catch (err) {
      retentionStatus = isApiError(err) ? err.message : 'Could not save the default.'
    } finally {
      retentionSaving = false
    }
  }

  // --- Cloud AI (§11.4) ---------------------------------------------------
  let cloudAiConfigured = $state(false)
  let cloudAiEndpointHost = $state<string | null>(null)
  let cloudAiModel = $state<string | null>(null)
  let cloudAiKeyLast4 = $state<string | null>(null)

  // Form inputs the user types to (re)configure Cloud AI. `cloudAiApiKeyInput` is
  // write-only: it exists only long enough to be sent on submit, then is reset to `''`
  // immediately after the `cloud_ai_set_config` call settles, whether it succeeded or
  // failed — nothing about the key persists in component state or the DOM past that point
  // (architecture §9.1; dev-plan W31's explicitly-named test).
  let cloudAiEndpointInput = $state('')
  let cloudAiModelInput = $state('')
  let cloudAiApiKeyInput = $state('')
  let cloudAiSaving = $state(false)
  let cloudAiError = $state('')
  let cloudAiTesting = $state(false)
  let cloudAiTestResult = $state('')
  let cloudAiConfirmingClear = $state(false)
  let cloudAiClearing = $state(false)

  async function refreshCloudAiConfig() {
    const out = await cloudAiGetConfig()
    cloudAiConfigured = out.configured
    cloudAiEndpointHost = out.endpoint_host
    cloudAiModel = out.model
    cloudAiKeyLast4 = out.key_last4
  }

  async function handleSaveCloudAi(event: SubmitEvent) {
    event.preventDefault()
    cloudAiError = ''
    cloudAiSaving = true
    try {
      await cloudAiSetConfig({
        endpoint_url: cloudAiEndpointInput,
        model: cloudAiModelInput,
        api_key: cloudAiApiKeyInput,
      })
      cloudAiEndpointInput = ''
      cloudAiModelInput = ''
      await refreshCloudAiConfig()
    } catch (err) {
      cloudAiError = isApiError(err) ? err.message : 'Could not save Cloud AI settings.'
    } finally {
      // Reset unconditionally: the key must not survive this call either way.
      cloudAiApiKeyInput = ''
      cloudAiSaving = false
    }
  }

  async function handleTestCloudAi() {
    cloudAiTestResult = ''
    cloudAiTesting = true
    try {
      const out = await cloudAiTest()
      cloudAiTestResult = out.ok
        ? 'Test succeeded.'
        : `Test failed (${out.error_class ?? 'unknown'}).`
    } catch (err) {
      cloudAiTestResult = isApiError(err) ? err.message : 'Could not run the test.'
    } finally {
      cloudAiTesting = false
    }
  }

  function handleRequestClearCloudAi() {
    cloudAiConfirmingClear = true
  }

  function handleCancelClearCloudAi() {
    cloudAiConfirmingClear = false
  }

  async function handleConfirmClearCloudAi() {
    cloudAiClearing = true
    try {
      await cloudAiClearConfig()
      await refreshCloudAiConfig()
    } finally {
      cloudAiClearing = false
      cloudAiConfirmingClear = false
    }
  }

  onMount(() => {
    getAccount().then((out) => {
      accountId = out.account_id
      displayName = out.display_name
      createdAt = out.created_at
    })
    getRetentionDefault().then((out) => {
      retentionPolicy = out.policy
    })
    refreshCloudAiConfig()
  })
</script>

<div class="screen">
  <AppShell active="settings" {onNavigateVault} onNavigateSettings={() => {}} {onLock} />

  <main>
    <section class="card">
      <h2>Account</h2>
      <dl>
        <dt>Display name</dt>
        <dd>{displayName}</dd>
        <dt>Account ID</dt>
        <dd>{accountId}</dd>
        <dt>Created</dt>
        <dd>{createdAtFormatted}</dd>
      </dl>
    </section>

    <section class="card">
      <h2>Passphrase</h2>
      <form onsubmit={handleChangePassphrase} novalidate>
        <div class="field">
          <label for="current-passphrase">Current passphrase</label>
          <input
            id="current-passphrase"
            type="password"
            bind:value={currentPassphrase}
            autocomplete="off"
          />
        </div>
        <div class="field">
          <label for="new-passphrase">New passphrase</label>
          <input
            id="new-passphrase"
            type="password"
            bind:value={newPassphrase}
            autocomplete="off"
          />
        </div>
        <div class="field">
          <label for="confirm-new-passphrase">Confirm new passphrase</label>
          <input
            id="confirm-new-passphrase"
            type="password"
            bind:value={confirmNewPassphrase}
            autocomplete="off"
          />
        </div>
        {#if passphraseError}
          <p class="field-error" role="alert">{passphraseError}</p>
        {/if}
        {#if passphraseSuccess}
          <p class="field-success">{passphraseSuccess}</p>
        {/if}
        <button type="submit" disabled={passphraseSubmitting}>Change passphrase</button>
      </form>
      <p class="recovery-copy">{NO_RECOVERY_COPY}</p>
    </section>

    <section class="card">
      <h2>Retention default</h2>
      <fieldset>
        <legend class="visually-hidden">Default for original files</legend>
        {#each RETENTION_POLICIES as policy (policy)}
          <label class="radio-row">
            <input
              type="radio"
              name="retention-policy"
              value={policy}
              checked={retentionPolicy === policy}
              onchange={() => (retentionPolicy = policy)}
            />
            {RETENTION_POLICY_LABELS[policy]}
          </label>
        {/each}
      </fieldset>
      {#if retentionStatus}
        <p class="field-success">{retentionStatus}</p>
      {/if}
      <button type="button" disabled={retentionSaving} onclick={handleSaveRetention}>
        Save default
      </button>
    </section>

    <section class="card">
      <h2>Cloud AI</h2>
      <p class="scope-copy">{CLOUD_AI_SCOPE_COPY}</p>

      <dl>
        <dt>Status</dt>
        <dd>{cloudAiConfigured ? 'Configured' : 'Not configured'}</dd>
        {#if cloudAiConfigured}
          <dt>Endpoint host</dt>
          <dd>{cloudAiEndpointHost}</dd>
          <dt>Model</dt>
          <dd>{cloudAiModel}</dd>
          <dt>API key</dt>
          <dd>ending {cloudAiKeyLast4}</dd>
        {/if}
      </dl>

      <form onsubmit={handleSaveCloudAi} novalidate>
        <div class="field">
          <label for="cloud-ai-endpoint">Endpoint (https)</label>
          <input
            id="cloud-ai-endpoint"
            type="text"
            bind:value={cloudAiEndpointInput}
            autocomplete="off"
          />
        </div>
        <div class="field">
          <label for="cloud-ai-model">Model ID</label>
          <input id="cloud-ai-model" type="text" bind:value={cloudAiModelInput} autocomplete="off" />
        </div>
        <div class="field">
          <label for="cloud-ai-api-key">API key</label>
          <input
            id="cloud-ai-api-key"
            type="password"
            bind:value={cloudAiApiKeyInput}
            autocomplete="off"
          />
        </div>
        {#if cloudAiError}
          <p class="field-error" role="alert">{cloudAiError}</p>
        {/if}
        <button type="submit" disabled={cloudAiSaving}>Save</button>
      </form>

      <div class="cloud-ai-actions">
        <button type="button" disabled={cloudAiTesting} onclick={handleTestCloudAi}>
          Test
        </button>
        {#if !cloudAiConfirmingClear}
          <button type="button" disabled={cloudAiClearing} onclick={handleRequestClearCloudAi}>
            Clear
          </button>
        {:else}
          <span class="confirm-clear">
            Clear the stored Cloud AI configuration?
            <button type="button" disabled={cloudAiClearing} onclick={handleConfirmClearCloudAi}>
              Yes, clear
            </button>
            <button type="button" disabled={cloudAiClearing} onclick={handleCancelClearCloudAi}>
              Cancel
            </button>
          </span>
        {/if}
      </div>
      {#if cloudAiTestResult}
        <p class="field-success">{cloudAiTestResult}</p>
      {/if}
    </section>
  </main>
</div>

<style>
  .screen {
    min-height: 100vh;
    background: var(--md-surface);
    font-family: var(--md-font);
  }

  main {
    padding: 32px 24px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    max-width: 560px;
    margin: 0 auto;
  }

  .card {
    padding: 24px;
    border-radius: var(--md-radius-xl);
    background: var(--md-surface-container-lowest);
    box-shadow: var(--md-elev-4);
  }

  h2 {
    margin: 0 0 16px;
    font-size: 16px;
    font-weight: 500;
    color: var(--md-on-surface);
  }

  dl {
    margin: 0;
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
  }

  dt {
    font-size: 12px;
    color: var(--md-on-surface-variant);
  }

  dd {
    margin: 0;
    font-size: 14px;
    color: var(--md-on-surface);
    word-break: break-all;
  }

  .field {
    margin-bottom: 16px;
  }

  label {
    display: block;
    font-size: 12px;
    color: var(--md-on-surface-variant);
    margin-bottom: 6px;
  }

  input[type='text'],
  input[type='password'] {
    width: 100%;
    box-sizing: border-box;
    height: 44px;
    padding: 0 14px;
    border-radius: var(--md-radius-sm);
    border: 1.5px solid var(--md-outline-variant);
    background: var(--md-surface-container-lowest);
    color: var(--md-on-surface);
    font-size: 15px;
    font-family: inherit;
  }

  input:focus {
    outline: none;
    border-color: var(--md-primary);
  }

  .radio-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    color: var(--md-on-surface);
    margin-bottom: 10px;
  }

  fieldset {
    border: none;
    padding: 0;
    margin: 0 0 12px;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }

  .field-error {
    margin: 6px 0 0;
    font-size: 11.5px;
    color: var(--md-error);
  }

  .field-success {
    margin: 6px 0 0;
    font-size: 11.5px;
    color: var(--md-on-surface-variant);
  }

  .recovery-copy {
    margin: 16px 0 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--md-on-surface-variant);
  }

  .scope-copy {
    margin: 0 0 16px;
    font-size: 12px;
    line-height: 1.6;
    color: var(--md-on-surface-variant);
  }

  .cloud-ai-actions {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 8px;
  }

  .confirm-clear {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--md-on-surface-variant);
  }

  button {
    height: 40px;
    padding: 0 16px;
    border: none;
    border-radius: var(--md-radius-full);
    background: var(--md-primary);
    color: var(--md-on-primary);
    font-size: 13.5px;
    font-weight: 500;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
