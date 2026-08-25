<script lang="ts">
  // Shared unlocked chrome (ui.md §4: "Chrome when unlocked: app name; primary nav Vault,
  // Audit trail, Settings; a Lock control always visible"). Extracted in W31 so Settings
  // and Vault share one header instead of forking W30's inline nav markup.
  //
  // W35: Audit trail is a real nav target (`AuditScreen`), matching Vault and Settings.

  let {
    active,
    onNavigateVault,
    onNavigateAudit,
    onNavigateSettings,
    onLock,
  }: {
    active: 'vault' | 'settings' | 'audit'
    onNavigateVault: () => void
    onNavigateAudit: () => void
    onNavigateSettings: () => void
    onLock: () => void
  } = $props()
</script>

<header>
  <span class="brand">Privacy Gate</span>
  <nav>
    <button
      type="button"
      class="nav-item"
      class:active={active === 'vault'}
      onclick={onNavigateVault}
    >
      Vault
    </button>
    <button
      type="button"
      class="nav-item"
      class:active={active === 'audit'}
      onclick={onNavigateAudit}
    >
      Audit trail
    </button>
    <button
      type="button"
      class="nav-item"
      class:active={active === 'settings'}
      onclick={onNavigateSettings}
    >
      Settings
    </button>
  </nav>
  <button type="button" class="lock-button" onclick={onLock}>Lock</button>
</header>

<style>
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
  }

  .nav-item {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: var(--md-on-surface-variant);
    cursor: pointer;
  }

  .nav-item.active {
    color: var(--md-primary);
    font-weight: 500;
  }

  .lock-button {
    height: 36px;
    padding: 0 16px;
    border: 1px solid var(--md-outline-variant);
    border-radius: var(--md-radius-full);
    background: var(--md-surface-container-lowest);
    color: var(--md-on-surface);
    font-size: 13px;
    cursor: pointer;
  }
</style>
