<script lang="ts">
  // ui.md §9 — named variants for one approved document. No in-place edit (design §3.4).
  // Save happens from the share override set (ShareScreen). This screen lists and deletes.

  import { onMount } from 'svelte'
  import AppShell from '../lib/AppShell.svelte'
  import { deleteVariant, isApiError, listVariants, type VariantSummary } from '../lib/api'
  import {
    DELETE_VARIANT_LABEL,
    VARIANT_NO_EDIT_COPY,
    VARIANTS_EMPTY_COPY,
    VARIANTS_TITLE,
  } from '../lib/copy'

  let {
    docId,
    sourceFilename,
    onLock,
    onNavigateVault,
    onNavigateSettings,
    onNavigateAudit,
  }: {
    docId: string
    sourceFilename: string
    onLock: () => void
    onNavigateVault: () => void
    onNavigateSettings: () => void
    onNavigateAudit: () => void
  } = $props()

  let variants = $state<VariantSummary[]>([])
  let loaded = $state(false)
  let loadError = $state('')
  let deleteConfirmId = $state<string | null>(null)
  let deleting = $state(false)

  async function refresh() {
    loadError = ''
    const out = await listVariants(docId)
    variants = out.variants
    loaded = true
  }

  async function confirmDelete(variantId: string) {
    deleting = true
    try {
      await deleteVariant(docId, variantId)
      deleteConfirmId = null
      await refresh()
    } catch (err) {
      loadError = isApiError(err) ? err.message : 'Could not delete the variant.'
    } finally {
      deleting = false
    }
  }

  function formatCreated(iso: string): string {
    const parsed = new Date(iso)
    if (Number.isNaN(parsed.getTime())) return iso
    return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(
      parsed,
    )
  }

  onMount(() => {
    refresh().catch((err) => {
      loadError = isApiError(err) ? err.message : 'Could not load variants.'
      loaded = true
    })
  })
</script>

<div class="screen">
  <AppShell
    active="vault"
    {onNavigateVault}
    {onNavigateAudit}
    {onNavigateSettings}
    {onLock}
  />

  <header class="topbar">
    <div>
      <h1>{VARIANTS_TITLE}</h1>
      {#if sourceFilename}
        <p class="filename">{sourceFilename}</p>
      {/if}
    </div>
  </header>

  {#if loadError}
    <p class="notice error" role="alert">{loadError}</p>
  {:else if loaded && variants.length === 0}
    <p class="notice" role="status">{VARIANTS_EMPTY_COPY}</p>
  {:else if variants.length > 0}
    <p class="hint">{VARIANT_NO_EDIT_COPY}</p>
    <ul class="list">
      {#each variants as variant (variant.variant_id)}
        <li>
          <div>
            <p class="name">{variant.name}</p>
            <p class="meta">{formatCreated(variant.created_at)}</p>
          </div>
          {#if deleteConfirmId === variant.variant_id}
            <div class="confirm">
              <button type="button" disabled={deleting} onclick={() => confirmDelete(variant.variant_id)}>
                Yes, delete
              </button>
              <button type="button" disabled={deleting} onclick={() => (deleteConfirmId = null)}>
                Cancel
              </button>
            </div>
          {:else}
            <button type="button" onclick={() => (deleteConfirmId = variant.variant_id)}>
              {DELETE_VARIANT_LABEL}
            </button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .screen {
    min-height: 100vh;
    background: var(--md-surface);
    font-family: var(--md-font);
    display: flex;
    flex-direction: column;
  }

  .topbar {
    padding: 12px 24px;
    border-bottom: 1px solid var(--md-outline-variant);
  }

  h1 {
    margin: 0;
    font-size: 24px;
    line-height: 32px;
    font-weight: 400;
  }

  .filename {
    margin: 2px 0 0;
    font-size: 12px;
    color: var(--md-on-surface-variant);
  }

  .notice,
  .hint {
    margin: 24px 32px;
    font-size: 14px;
    line-height: 20px;
    color: var(--md-on-surface-variant);
    max-width: 640px;
  }

  .notice.error {
    color: var(--md-error);
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 16px 32px 32px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 16px 20px;
    background: var(--md-surface-container-lowest);
    border: 1px solid var(--md-outline-variant);
    border-radius: var(--md-radius-md);
  }

  .name {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }

  .meta {
    margin: 4px 0 0;
    font-size: 12px;
    color: var(--md-on-surface-variant);
  }

  .confirm {
    display: flex;
    gap: 8px;
  }

  button {
    height: 36px;
    padding: 0 14px;
    border-radius: var(--md-radius-full);
    border: 1px solid var(--md-outline);
    background: transparent;
    color: var(--md-on-surface);
    font-size: 13px;
    cursor: pointer;
  }
</style>
