<script lang="ts">
  // Placeholder only (dev-plan W30: "No vault list yet (empty state ok)"). Just enough to
  // prove the router landed somewhere real for the `unlocked` state. Real vault-list logic
  // is a later chunk. W31 moved the nav/Lock chrome into `AppShell` so this screen and
  // `SettingsScreen` share one header instead of each carrying its own copy.

  import AppShell from '../lib/AppShell.svelte'

  let { onLock, onNavigateSettings }: { onLock: () => void; onNavigateSettings: () => void } =
    $props()
</script>

<div class="screen">
  <AppShell
    active="vault"
    onNavigateVault={() => {}}
    {onNavigateSettings}
    {onLock}
  />

  <main>
    <p class="empty-state">No documents yet. Import a document to get started.</p>
  </main>
</div>

<style>
  .screen {
    min-height: 100vh;
    background: var(--md-surface);
    font-family: var(--md-font);
  }

  main {
    padding: 48px 24px;
    display: flex;
    justify-content: center;
  }

  .empty-state {
    color: var(--md-on-surface-variant);
    font-size: 14px;
  }
</style>
