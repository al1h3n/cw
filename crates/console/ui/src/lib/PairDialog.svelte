<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { onMount } from 'svelte'
  import { t } from './i18n.svelte'
  import type { PairingInvite } from './types'

  let { onclose }: { onclose: () => void } = $props()

  let invite = $state<PairingInvite | null>(null)
  let added = $state<string | null>(null)
  let error = $state<string | null>(null)
  let copied = $state(false)

  onMount(async () => {
    try {
      invite = await invoke<PairingInvite>('begin_pairing')
      // Resolves when a student PC dials in with the code, or rejects on timeout/refusal.
      added = await invoke<string>('await_pairing')
    } catch (e) {
      error = String(e)
    }
  })

  async function copyCommand() {
    if (!invite) return
    await navigator.clipboard.writeText(invite.command)
    copied = true
    setTimeout(() => (copied = false), 1500)
  }
</script>

<div
  class="backdrop"
  role="presentation"
  onclick={(e) => e.target === e.currentTarget && onclose()}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('pairTitle')}>
    <h2>{t('pairTitle')}</h2>

    {#if error}
      <p class="error">{error}</p>
      <div class="row"><button class="primary" onclick={onclose}>{t('close')}</button></div>
    {:else if added}
      <p class="done">{t('pairDone', added)}</p>
      <div class="row"><button class="primary" onclick={onclose}>{t('close')}</button></div>
    {:else if invite}
      <p class="step">{t('pairStep1')}</p>
      <div class="command">
        <code>{invite.command}</code>
        <button onclick={copyCommand}>{copied ? t('copied') : t('copy')}</button>
      </div>

      <p class="step">{t('pairStep2')}</p>
      <p class="code">{invite.code}</p>

      <div class="row waiting">
        <span class="spinner" aria-hidden="true"></span>
        <span>{t('pairWaiting')}</span>
        <button onclick={onclose}>{t('cancel')}</button>
      </div>
    {:else}
      <p class="step">{t('loading')}</p>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(6, 9, 13, 0.66);
    padding: 20px;
  }

  .dialog {
    width: min(560px, 100%);
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 14px;
    padding: 22px;
  }

  h2 {
    margin: 0 0 14px;
    font-size: 17px;
  }

  .step {
    margin: 14px 0 8px;
    color: var(--muted);
    font-size: 13px;
  }

  .command {
    display: flex;
    align-items: center;
    gap: 10px;
    background: #0c1015;
    border: 1px solid var(--line);
    border-radius: var(--radius);
    padding: 10px 12px;
  }

  .command code {
    flex: 1;
    font-size: 12px;
    word-break: break-all;
    color: #cfe0ff;
  }

  .code {
    margin: 0;
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
    font-size: 38px;
    letter-spacing: 8px;
    text-align: center;
    padding: 10px 0 4px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 16px;
  }

  .waiting {
    color: var(--muted);
    font-size: 13px;
  }

  .waiting button {
    margin-left: auto;
  }

  .done {
    color: var(--live);
    margin: 10px 0 0;
  }

  .error {
    color: #ff9d9d;
    margin: 10px 0 0;
    font-size: 13px;
  }

  .spinner {
    width: 13px;
    height: 13px;
    border: 2px solid #3a4453;
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.9s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
</style>
