<script lang="ts">
  import { onMount } from 'svelte'
  import { listen, type UnlistenFn } from '@tauri-apps/api/event'
  import {
    getSessionState,
    lock,
    SESSION_CHANGED_EVENT,
    type CreateAccountOut,
    type IntegrityReport,
    type SessionState,
    type SessionStateOut,
    type UnlockOut,
  } from './lib/api'
  import FirstRunScreen from './screens/FirstRunScreen.svelte'
  import UnlockScreen from './screens/UnlockScreen.svelte'
  import IntegrityScreen from './screens/IntegrityScreen.svelte'
  import VaultScreen from './screens/VaultScreen.svelte'
  import SettingsScreen from './screens/SettingsScreen.svelte'

  // ui.md §4: no client-side router for four screens — plain reactive state switching on
  // `SessionState` is enough. ui.md §14: chrome first paint is static and gated on
  // `get_session_state` only, so this is the one command `onMount` awaits before any
  // screen renders.
  let sessionState = $state<SessionState | null>(null)
  let integrity = $state<IntegrityReport | null>(null)

  // W31: Settings is a sub-view of the `unlocked` state, not a `SessionState` of its own
  // (api.md §2 has no such state) — plain local view state, same "no router" reasoning as
  // above. Reset to 'vault' on every fresh unlock and on lock so re-entering the vault
  // never leaves Settings showing stale data.
  let view = $state<'vault' | 'settings'>('vault')

  onMount(() => {
    let unlisten: UnlistenFn | undefined

    getSessionState()
      .then((out) => {
        sessionState = out.state
      })
      .catch(() => {
        // get_session_state has no gate and no documented failure mode; if the IPC call
        // itself fails there is nothing more specific to show than staying on the loading
        // state indefinitely, which is preferable to guessing a session state.
      })

    // W29 already emits this event; wiring the listener is cheap and keeps `sessionState`
    // in sync with an out-of-band transition (api.md §6). It intentionally never sets
    // `integrity`: the event payload (`SessionStateOut`) carries no `IntegrityReport`, so
    // the `unlock` → `degraded_integrity` transition itself is still handled by
    // `handleUnlocked`'s direct invoke response, which does carry the report.
    listen<SessionStateOut>(SESSION_CHANGED_EVENT, (event) => {
      sessionState = event.payload.state
    })
      .then((fn) => {
        unlisten = fn
      })
      .catch(() => {
        // No event surface in this test/dev environment; direct invoke responses already
        // drive navigation, so this is not fatal.
      })

    return () => {
      unlisten?.()
    }
  })

  // ui.md §3.3: window title is "Privacy Gate" or "Privacy Gate — Locked" only — never
  // document content.
  $effect(() => {
    document.title = sessionState === 'locked' ? 'Privacy Gate — Locked' : 'Privacy Gate'
  })

  function handleAccountCreated(out: CreateAccountOut) {
    sessionState = out.state
  }

  function handleUnlocked(out: UnlockOut) {
    sessionState = out.state
    integrity = out.integrity
    view = 'vault'
  }

  async function handleLock() {
    const out = await lock()
    sessionState = out.state
    integrity = null
    view = 'vault'
  }
</script>

{#if sessionState === 'first_run'}
  <FirstRunScreen onSuccess={handleAccountCreated} />
{:else if sessionState === 'locked'}
  <UnlockScreen onUnlocked={handleUnlocked} />
{:else if sessionState === 'degraded_integrity'}
  <IntegrityScreen onLock={handleLock} />
{:else if sessionState === 'unlocked'}
  {#if view === 'vault'}
    <VaultScreen onLock={handleLock} onNavigateSettings={() => (view = 'settings')} />
  {:else}
    <SettingsScreen onLock={handleLock} onNavigateVault={() => (view = 'vault')} />
  {/if}
{/if}
