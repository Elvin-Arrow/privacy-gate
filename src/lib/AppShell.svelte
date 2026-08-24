<script lang="ts">
  // Shared unlocked chrome (ui.md §4: "Chrome when unlocked: app name; primary nav Vault,
  // Audit trail, Settings; a Lock control always visible"). Extracted in W31 so Settings
  // and Vault share one header instead of forking W30's inline nav markup.
  //
  // Audit trail is a later chunk (W3x+); rendering it as a real nav target here would fake
  // content that doesn't exist yet, so it stays a non-interactive label (not a button, not
  // a link) until that screen lands — see docs/dev-log/0043-w31-ui-settings.md.

  let {
    active,
    onNavigateVault,
    onNavigateSettings,
    onLock,
  }: {
    active: 'vault' | 'settings'
    onNavigateVault: () => void
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
    <span class="nav-item disabled">Audit trail</span>
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

  .nav-item.disabled {
    opacity: 0.5;
    cursor: default;
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
