<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { t } from './i18n.svelte'

  /**
   * Lock and power controls. `deviceId` targets one PC; `null` targets every connected PC, which is
   * why the destructive actions always ask once more and say how many PCs they will hit.
   */
  let {
    deviceId,
    liveCount,
    onerror,
    onsent,
  }: {
    deviceId: string | null
    liveCount: number
    onerror: (message: string) => void
    onsent?: (count: number) => void
  } = $props()

  type PowerAction = 'shutdown' | 'reboot' | 'log-off'

  let open = $state(false)
  let delay = $state(60)
  let confirming = $state<PowerAction | null>(null)

  const disabled = $derived(liveCount === 0)
  const room = $derived(deviceId === null)

  async function send(action: string, delaySeconds = 0) {
    try {
      const count = await invoke<number>('perform', { deviceId, action, delaySeconds })
      onsent?.(count)
    } catch (e) {
      onerror(String(e))
    }
    confirming = null
    open = false
  }

  function label(action: PowerAction): string {
    return action === 'shutdown' ? t('actShutdown') : action === 'reboot' ? t('actReboot') : t('actLogOff')
  }

  function onkey(event: KeyboardEvent) {
    if (event.key === 'Escape' && open) {
      // Escape backs out one step at a time.
      event.stopPropagation()
      if (confirming) confirming = null
      else open = false
    }
  }
</script>

<!-- Escape is handled here, not on the window, so it stops before reaching the opened screen. -->
<span class="wrap" role="presentation" onkeydown={onkey}>
  <button onclick={() => send('lock-screen')} {disabled} title={room ? t('lockAllHint') : t('lockHint')}>
    {room ? t('actLockAll') : t('actLock')}
  </button>
  <button onclick={() => ((open = !open), (confirming = null))} {disabled} aria-expanded={open}>
    {room ? t('actPowerAll') : t('actPower')} ▾
  </button>

  {#if open}
    <div class="menu" role="menu">
      {#if confirming}
        <p class="question">
          {room ? t('confirmRoom', label(confirming), liveCount) : t('confirmOne', label(confirming))}
        </p>
        <p class="note">{delay === 0 ? t('delayNowNote') : t('delayNote', delay)}</p>
        <div class="row">
          <button class="danger" onclick={() => confirming && send(confirming, delay)}>
            {label(confirming)}
          </button>
          <button onclick={() => (confirming = null)}>{t('cancel')}</button>
        </div>
      {:else}
        <div class="delays" role="radiogroup" aria-label={t('delay')}>
          {#each [0, 60, 300] as seconds (seconds)}
            <button
              class="chip"
              class:active={delay === seconds}
              role="radio"
              aria-checked={delay === seconds}
              onclick={() => (delay = seconds)}
            >
              {seconds === 0 ? t('delayNow') : t('delayMinutes', seconds / 60)}
            </button>
          {/each}
        </div>
        <!-- Custom timer: type any number of minutes; the chosen value drives the confirmation note. -->
        <label class="custom">
          <span>{t('delayCustom')}</span>
          <input
            type="number"
            min="0"
            max="600"
            value={Math.round(delay / 60)}
            oninput={(e) =>
              (delay = Math.max(0, Math.min(600, Number((e.currentTarget as HTMLInputElement).value))) * 60)}
          />
          <span class="unit">{t('minutesUnit')}</span>
        </label>
        {#each ['shutdown', 'reboot', 'log-off'] as const as action (action)}
          <button class="item" role="menuitem" onclick={() => (confirming = action)}>{label(action)}</button>
        {/each}
        <hr />
        <button class="item" role="menuitem" onclick={() => send('cancel-shutdown')}>{t('actCancel')}</button>
      {/if}
    </div>
  {/if}
</span>

<style>
  .wrap {
    position: relative;
    display: inline-flex;
    gap: 8px;
  }

  .menu {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 20;
    display: grid;
    gap: 4px;
    width: 250px;
    padding: 8px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
  }

  .item {
    text-align: left;
    background: transparent;
    border-color: transparent;
  }

  .item:hover {
    border-color: var(--line);
  }

  .delays {
    display: flex;
    gap: 6px;
    padding-bottom: 4px;
  }

  .custom {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 2px 6px;
    color: var(--muted);
    font-size: 12px;
  }

  .custom input {
    width: 4.5em;
    padding: 4px 6px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--text);
    font: inherit;
    font-size: 12px;
  }

  .chip {
    flex: 1;
    padding: 4px 6px;
    font-size: 11.5px;
    border-radius: 999px;
    background: transparent;
  }

  .chip.active {
    background: var(--accent);
    border-color: var(--accent);
    color: #06101f;
    font-weight: 600;
  }

  hr {
    width: 100%;
    margin: 2px 0;
    border: 0;
    border-top: 1px solid var(--line);
  }

  .question {
    margin: 2px 2px 0;
    font-size: 13px;
  }

  .note {
    margin: 0 2px 6px;
    color: var(--muted);
    font-size: 12px;
  }

  .row {
    display: flex;
    gap: 8px;
  }

  .danger {
    background: var(--danger);
    border-color: var(--danger);
    color: #1d0606;
    font-weight: 600;
  }
</style>
