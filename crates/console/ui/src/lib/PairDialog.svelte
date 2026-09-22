<script lang="ts">
  import { invoke } from './bridge'
  import { listen, type UnlistenFn } from './bridge'
  import { onMount, onDestroy } from 'svelte'
  import { fly } from 'svelte/transition'
  import { flip } from 'svelte/animate'
  import { t } from './i18n.svelte'
  import { toasts } from './toast-store.svelte'
  import type { PairingInvite } from './types'

  let { onclose }: { onclose: () => void } = $props()

  let invite = $state<PairingInvite | null>(null)
  let joined = $state<string[]>([])
  let error = $state<string | null>(null)
  let copied = $state(false)
  let unlisten: UnlistenFn[] = []

  onMount(async () => {
    try {
      // One code, one open panel: PCs keep joining until the teacher clicks Done.
      unlisten.push(
        await listen<string>('cowatcher://paired', (e) => {
          if (!joined.includes(e.payload)) joined.unshift(e.payload)
          toasts.push(t('pairJoined', e.payload), 'ok')
        }),
      )
      unlisten.push(
        await listen<string>('cowatcher://pair-error', (e) => {
          // A wrong code from one PC should not stop the panel; surface it quietly.
          toasts.push(e.payload, 'error')
        }),
      )
      invite = await invoke<PairingInvite>('begin_pairing')
    } catch (e) {
      error = String(e)
    }
  })

  onDestroy(() => {
    for (const un of unlisten) un()
    invoke('stop_pairing').catch(() => {})
  })

  async function copyCommand() {
    if (!invite) return
    await navigator.clipboard.writeText(invite.command)
    copied = true
    setTimeout(() => (copied = false), 1500)
  }
</script>

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('pairTitle')}>
    <h2>{t('pairTitle')}</h2>

    {#if error}
      <p class="error">{error}</p>
      <div class="row"><button class="primary" onclick={onclose}>{t('close')}</button></div>
    {:else if invite}
      <p class="step">{t('pairStep1')}</p>
      <div class="command">
        <code>{invite.command}</code>
        <button onclick={copyCommand}>{copied ? t('copied') : t('copy')}</button>
      </div>

      <p class="step">{t('pairStep2')}</p>
      <p class="code">{invite.code}</p>
      <p class="hint">{t('pairKeepOpen')}</p>

      {#if joined.length > 0}
        <ul class="joined">
          {#each joined as id (id)}
            <li in:fly={{ y: -6, duration: 150 }} animate:flip={{ duration: 160 }}>
              <i class="ok"></i>
              <code>{id}</code>
            </li>
          {/each}
        </ul>
      {/if}

      <div class="row waiting">
        <span class="spinner" aria-hidden="true"></span>
        <span>{joined.length > 0 ? t('pairAddMore', joined.length) : t('pairWaiting')}</span>
        <button class="primary" onclick={onclose}>{t('done')}</button>
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
    max-height: 90vh;
    overflow: auto;
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

  .hint {
    margin: 0 0 6px;
    text-align: center;
    color: var(--muted);
    font-size: 12px;
  }

  .joined {
    list-style: none;
    margin: 8px 0 0;
    padding: 8px;
    max-height: 30vh;
    overflow: auto;
    display: grid;
    gap: 5px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 10px;
  }

  .joined li {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }

  .joined code {
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
    letter-spacing: 1px;
  }

  .joined .ok {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--live);
    flex-shrink: 0;
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
