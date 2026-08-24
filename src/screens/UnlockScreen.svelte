<script lang="ts">
  import { isApiError, unlock, type UnlockOut } from '../lib/api'
  import { NO_RECOVERY_COPY, UNLOCK_FAILED_COPY } from '../lib/copy'

  // ui.md §5.2 / §13.2: no "forgot passphrase" control anywhere on this screen — only the
  // C-ARCH-7 non-recovery sentence, never a link or button.

  let { onUnlocked }: { onUnlocked: (out: UnlockOut) => void } = $props()

  let passphrase = $state('')
  let submitting = $state(false)
  let errorMessage = $state('')

  async function handleSubmit(event: SubmitEvent) {
    event.preventDefault()
    errorMessage = ''
    submitting = true
    try {
      const out = await unlock({ passphrase })
      onUnlocked(out)
    } catch (err) {
      // ui.md §15: `unlock_failed` uses the canonical copy, not the core's internal
      // (non-secret but non-canonical) "unlock failed" class string.
      errorMessage =
        isApiError(err) && err.code === 'unlock_failed'
          ? UNLOCK_FAILED_COPY
          : isApiError(err)
            ? err.message
            : 'Could not unlock.'
    } finally {
      submitting = false
    }
  }
</script>

<div class="screen">
  <div class="card">
    <div class="brand">Privacy Gate</div>
    <h1>Unlock your vault</h1>
    <p class="subtitle">Enter your passphrase to decrypt and open your documents.</p>

    <form onsubmit={handleSubmit} novalidate>
      <div class="field">
        <label for="unlock-passphrase">Passphrase</label>
        <input
          id="unlock-passphrase"
          type="password"
          bind:value={passphrase}
          autocomplete="off"
        />
      </div>

      {#if errorMessage}
        <p class="field-error" role="alert">{errorMessage}</p>
      {/if}

      <button type="submit" disabled={submitting}>Unlock</button>
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
    max-width: 400px;
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
    height: 48px;
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

  .field-error {
    margin: 6px 0 16px;
    font-size: 12px;
    color: var(--md-error);
  }

  button {
    width: 100%;
    height: 48px;
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
