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
  import ApprovalScreen from './screens/ApprovalScreen.svelte'
  import ShareScreen from './screens/ShareScreen.svelte'
  import AuditScreen from './screens/AuditScreen.svelte'
  import VariantsScreen from './screens/VariantsScreen.svelte'

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
  let view = $state<'vault' | 'settings' | 'approval' | 'share' | 'audit' | 'variants'>('vault')
  let approvalDocId = $state<string | null>(null)
  let approvalFilename = $state('')
  let shareDocId = $state<string | null>(null)
  let shareFilename = $state('')
  let variantsDocId = $state<string | null>(null)
  let variantsFilename = $state('')

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

    // W35: `pg://session-changed` must land on the integrity screen (dev-plan named
    // integrate). The event payload is still only `{ state }` — Save report fetches
    // `get_integrity_report` itself, so we do not invent a report here.
    listen<SessionStateOut>(SESSION_CHANGED_EVENT, (event) => {
      sessionState = event.payload.state
      if (event.payload.state === 'degraded_integrity') {
        view = 'vault'
        approvalDocId = null
        approvalFilename = ''
        shareDocId = null
        shareFilename = ''
        variantsDocId = null
        variantsFilename = ''
      }
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
    approvalDocId = null
    approvalFilename = ''
    shareDocId = null
    shareFilename = ''
    variantsDocId = null
    variantsFilename = ''
  }

  function handleOpenApproval(docId: string, sourceFilename: string) {
    approvalDocId = docId
    approvalFilename = sourceFilename
    view = 'approval'
  }

  function handleOpenShare(docId: string, sourceFilename: string) {
    shareDocId = docId
    shareFilename = sourceFilename
    view = 'share'
  }

  function handleOpenVariants(docId: string, sourceFilename: string) {
    variantsDocId = docId
    variantsFilename = sourceFilename
    view = 'variants'
  }

  function handleApprovalDone() {
    approvalDocId = null
    approvalFilename = ''
    view = 'vault'
  }

  function handleShareDone() {
    shareDocId = null
    shareFilename = ''
    view = 'vault'
  }

  function handleNavigateAudit() {
    approvalDocId = null
    approvalFilename = ''
    shareDocId = null
    shareFilename = ''
    variantsDocId = null
    variantsFilename = ''
    view = 'audit'
  }

  async function handleLock() {
    const out = await lock()
    sessionState = out.state
    integrity = null
    view = 'vault'
    approvalDocId = null
    approvalFilename = ''
    shareDocId = null
    shareFilename = ''
    variantsDocId = null
    variantsFilename = ''
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
    <VaultScreen
      onLock={handleLock}
      onNavigateSettings={() => (view = 'settings')}
      onNavigateAudit={handleNavigateAudit}
      onOpenApproval={handleOpenApproval}
      onOpenShare={handleOpenShare}
      onOpenVariants={handleOpenVariants}
    />
  {:else if view === 'approval' && approvalDocId}
    <ApprovalScreen
      docId={approvalDocId}
      sourceFilename={approvalFilename}
      onLock={handleLock}
      onNavigateVault={handleApprovalDone}
      onNavigateSettings={() => {
        approvalDocId = null
        approvalFilename = ''
        view = 'settings'
      }}
      onNavigateAudit={handleNavigateAudit}
      onDone={handleApprovalDone}
    />
  {:else if view === 'share' && shareDocId}
    <ShareScreen
      docId={shareDocId}
      sourceFilename={shareFilename}
      onLock={handleLock}
      onNavigateVault={handleShareDone}
      onNavigateSettings={() => {
        shareDocId = null
        shareFilename = ''
        view = 'settings'
      }}
      onNavigateAudit={handleNavigateAudit}
      onDone={handleShareDone}
    />
  {:else if view === 'audit'}
    <AuditScreen
      onLock={handleLock}
      onNavigateVault={() => (view = 'vault')}
      onNavigateSettings={() => (view = 'settings')}
    />
  {:else if view === 'variants' && variantsDocId}
    <VariantsScreen
      docId={variantsDocId}
      sourceFilename={variantsFilename}
      onLock={handleLock}
      onNavigateVault={() => {
        variantsDocId = null
        variantsFilename = ''
        view = 'vault'
      }}
      onNavigateSettings={() => {
        variantsDocId = null
        variantsFilename = ''
        view = 'settings'
      }}
      onNavigateAudit={handleNavigateAudit}
    />
  {:else}
    <SettingsScreen
      onLock={handleLock}
      onNavigateVault={() => (view = 'vault')}
      onNavigateAudit={handleNavigateAudit}
    />
  {/if}
{/if}
