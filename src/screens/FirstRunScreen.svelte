<script lang="ts">
  import { createAccount, isApiError, type CreateAccountOut } from '../lib/api'
  import { NO_RECOVERY_COPY } from '../lib/copy'

  const MIN_PASSPHRASE_LENGTH = 8

  let { onSuccess }: { onSuccess: (out: CreateAccountOut) => void } = $props()

  let displayName = $state('')
  let passphrase = $state('')
  let confirmPassphrase = $state('')
  let submitting = $state(false)
  let nameError = $state('')
  let passphraseError = $state('')
  let confirmError = $state('')
  let apiErrorMessage = $state('')

  function validate(): boolean {
    nameError = ''
    passphraseError = ''
    confirmError = ''

    const trimmedName = displayName.trim()
    if (trimmedName.length === 0) {
      nameError = 'Enter a display name.'
    }
    if (passphrase.length < MIN_PASSPHRASE_LENGTH) {
      passphraseError = `Passphrase must be at least ${MIN_PASSPHRASE_LENGTH} characters.`
    }
    if (passphrase !== confirmPassphrase) {
      confirmError = "Passphrases don't match."
    }

    return nameError === '' && passphraseError === '' && confirmError === ''
  }

  async function handleSubmit(event: SubmitEvent) {
    event.preventDefault()
    apiErrorMessage = ''
    if (!validate()) {
      return
    }
    submitting = true
    try {
      const out = await createAccount({
        display_name: displayName.trim(),
        passphrase,
      })
      onSuccess(out)
    } catch (err) {
      apiErrorMessage = isApiError(err) ? err.message : 'Could not create the vault.'
    } finally {
      submitting = false
    }
  }
</script>

<div class="screen">
  <div class="card">
    <div class="brand">Privacy Gate</div>
    <h1>Create your vault</h1>
    <p class="subtitle">
      Set a display name and a passphrase. Everything you import is encrypted with it.
    </p>

    <form onsubmit={handleSubmit} novalidate>
      <div class="field">
        <label for="display-name">Display name</label>
        <input id="display-name" type="text" bind:value={displayName} autocomplete="off" />
        {#if nameError}
          <p class="field-error" role="alert">{nameError}</p>
        {/if}
      </div>

      <div class="field">
        <label for="passphrase">Passphrase</label>
        <input
          id="passphrase"
          type="password"
          bind:value={passphrase}
          autocomplete="off"
        />
        <p class="hint">At least {MIN_PASSPHRASE_LENGTH} characters. Longer passphrases are stronger.</p>
        {#if passphraseError}
          <p class="field-error" role="alert">{passphraseError}</p>
        {/if}
      </div>

      <div class="field">
        <label for="confirm-passphrase">Confirm passphrase</label>
        <input
          id="confirm-passphrase"
          type="password"
          bind:value={confirmPassphrase}
          autocomplete="off"
        />
        {#if confirmError}
          <p class="field-error" role="alert">{confirmError}</p>
        {/if}
      </div>

      {#if apiErrorMessage}
        <p class="field-error" role="alert">{apiErrorMessage}</p>
      {/if}

      <button type="submit" disabled={submitting}>Create your vault</button>
    </form>

    <p class="recovery-copy">{NO_RECOVERY_COPY}</p>
  </div>
</div>

<style>
  .screen {
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--md-surface);
    font-family: var(--md-font);
  }

  .card {
    width: 100%;
    max-width: 420px;
    padding: 40px;
    border-radius: var(--md-radius-xl);
    background: var(--md-surface-container-lowest);
    box-shadow: var(--md-elev-4);
  }

  .brand {
    font-weight: 500;
    font-size: 14px;
    color: var(--md-on-surface-variant);
    margin-bottom: 16px;
  }

  h1 {
    margin: 0 0 8px;
    font-size: 26px;
    font-weight: 500;
    color: var(--md-on-surface);
  }

  .subtitle {
    margin: 0 0 24px;
    color: var(--md-on-surface-variant);
    font-size: 14px;
    line-height: 1.5;
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

  input {
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

  .hint {
    margin: 6px 0 0;
    font-size: 11.5px;
    color: var(--md-on-surface-variant);
  }

  .field-error {
    margin: 6px 0 0;
    font-size: 11.5px;
    color: var(--md-error);
  }

  button {
    width: 100%;
    height: 48px;
    margin-top: 8px;
    border: none;
    border-radius: var(--md-radius-full);
    background: var(--md-primary);
    color: var(--md-on-primary);
    font-size: 15px;
    font-weight: 500;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .recovery-copy {
    margin: 20px 0 0;
    font-size: 12px;
    line-height: 1.6;
    color: var(--md-on-surface-variant);
  }
</style>
