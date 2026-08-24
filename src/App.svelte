<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { onMount } from 'svelte'

  let loading = true
  let sessionState: string | null = null
  let error: string | null = null

  async function getSessionState() {
    try {
      const result = await invoke<{ state: string }>('get_session_state')
      sessionState = result.state
    } catch (err) {
      error = `Failed to get session state: ${err}`
    } finally {
      loading = false
    }
  }

  onMount(() => {
    getSessionState()
  })
</script>

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    font-family: -apple-system, 'Segoe UI', sans-serif;
  }

  .container {
    width: 100%;
    height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #f5f5f5;
  }

  .content {
    text-align: center;
    padding: 2rem;
    background: white;
    border-radius: 8px;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
    max-width: 600px;
  }

  h1 {
    margin: 0 0 1rem 0;
    color: #333;
  }

  p {
    margin: 0.5rem 0;
    color: #666;
  }

  .error {
    color: #d32f2f;
    padding: 1rem;
    background: #ffebee;
    border-radius: 4px;
    margin-top: 1rem;
  }

  .loading {
    color: #999;
  }
</style>

<div class="container">
  <div class="content">
    <h1>Privacy Gate</h1>
    {#if loading}
      <p class="loading">Initializing...</p>
    {:else if error}
      <p class="error">{error}</p>
    {:else}
      <p>Session state: <strong>{sessionState}</strong></p>
      <p>App is ready.</p>
    {/if}
  </div>
</div>
