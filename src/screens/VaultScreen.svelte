<script lang="ts">
  // Placeholder only (dev-plan W30: "No vault list yet (empty state ok)"). Just enough to
  // prove the router landed somewhere real for the `unlocked` state, and to give the Lock
  // control §5.3 requires from every unlocked chrome. The real vault list is a later chunk.

  let { onLock }: { onLock: () => void } = $props()
</script>

<div class="screen">
  <header>
    <span class="brand">Privacy Gate</span>
    <nav>
      <span class="nav-item active">Vault</span>
      <span class="nav-item">Audit trail</span>
      <span class="nav-item">Settings</span>
    </nav>
    <button type="button" onclick={onLock}>Lock</button>
  </header>

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

  header {
    display: flex;
    align-items: center;
    gap: 24px;
    padding: 16px 24px;
    border-bottom: 1px solid var(--md-outline-variant);
  }

  .brand {
    font-weight: 500;
    font-size: 15px;
    color: var(--md-on-surface);
  }

  nav {
    display: flex;
    gap: 16px;
    flex: 1;
    font-size: 13px;
    color: var(--md-on-surface-variant);
  }

  .nav-item.active {
    color: var(--md-primary);
    font-weight: 500;
  }

  header button {
    height: 36px;
    padding: 0 16px;
    border: 1px solid var(--md-outline-variant);
    border-radius: var(--md-radius-full);
    background: var(--md-surface-container-lowest);
    color: var(--md-on-surface);
    font-size: 13px;
    cursor: pointer;
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
