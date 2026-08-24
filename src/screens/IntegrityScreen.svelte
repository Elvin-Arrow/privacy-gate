<script lang="ts">
  import { save } from '@tauri-apps/plugin-dialog'
  import { writeTextFile } from '@tauri-apps/plugin-fs'
  import { getIntegrityReport, isApiError } from '../lib/api'
  import { INTEGRITY_BODY, INTEGRITY_REPORT_FILENAME, INTEGRITY_TITLE } from '../lib/copy'

  // ui.md §13.1: full-screen, fail-closed. Only two actions exist here — Save report and
  // Lock. There is deliberately no "open anyway" / "open documents" control anywhere in
  // this component (C-UI-5); do not add one.

  let { onLock }: { onLock: () => void } = $props()

  let saveStatus = $state('')
  let saving = $state(false)

  async function handleSaveReport() {
    saveStatus = ''
    saving = true
    try {
      const report = await getIntegrityReport()
      // ui.md §10.4 sequence, applied to the §13.1 "same save-dialog rules" note: open the
      // OS save dialog first; a cancel (`path` is `null`) does nothing further.
      const path = await save({
        defaultPath: INTEGRITY_REPORT_FILENAME,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      })
      if (!path) {
        return
      }
      await writeTextFile(path, JSON.stringify(report, null, 2))
      saveStatus = 'Report saved.'
    } catch (err) {
      saveStatus = isApiError(err) ? err.message : 'Could not save the report.'
    } finally {
      saving = false
    }
  }
</script>

<div class="screen">
  <div class="card">
    <h1>{INTEGRITY_TITLE}</h1>
    <p class="body">{INTEGRITY_BODY}</p>

    {#if saveStatus}
      <p class="status" role="status">{saveStatus}</p>
    {/if}

    <div class="actions">
      <button type="button" onclick={handleSaveReport} disabled={saving}>Save report</button>
      <button type="button" class="lock" onclick={onLock}>Lock</button>
    </div>
  </div>
</div>

<style>
  .screen {
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--md-error-container);
    font-family: var(--md-font);
    padding: 24px;
    box-sizing: border-box;
  }

  .card {
    width: 100%;
    max-width: 480px;
    padding: 40px;
    border-radius: var(--md-radius-xl);
    background: var(--md-surface-container-lowest);
    box-shadow: var(--md-elev-4);
  }

  h1 {
    margin: 0 0 16px;
    font-size: 22px;
    font-weight: 500;
    color: var(--md-on-error-container);
  }

  .body {
    margin: 0 0 24px;
    font-size: 14px;
    line-height: 1.6;
    color: var(--md-on-surface);
  }

  .status {
    margin: 0 0 16px;
    font-size: 13px;
    color: var(--md-on-surface-variant);
  }

  .actions {
    display: flex;
    gap: 12px;
  }

  button {
    flex: 1;
    height: 44px;
    border: none;
    border-radius: var(--md-radius-full);
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
  }

  button:not(.lock) {
    background: var(--md-primary);
    color: var(--md-on-primary);
  }

  button.lock {
    background: var(--md-surface-container);
    color: var(--md-on-surface);
    border: 1px solid var(--md-outline-variant);
  }

  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
