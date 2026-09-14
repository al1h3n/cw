<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { t } from './i18n.svelte'
  import type { RecordingInfo } from './types'

  /** Start/stop recording for one PC, with the size and rate the teacher picks. */
  let { deviceId, onerror }: { deviceId: string; onerror: (message: string) => void } = $props()

  /** Sensible presets. The PC clamps anything it cannot deliver and tells us what it really did. */
  const PRESETS = [
    { label: '720p', width: 1280, height: 720 },
    { label: '1080p', width: 1920, height: 1080 },
    { label: '1440p', width: 2560, height: 1440 },
  ]
  const RATES = [5, 10, 30]

  let info = $state<RecordingInfo | null>(null)
  let open = $state(false)
  let busy = $state(false)
  let preset = $state(0)
  let fps = $state(10)

  const recording = $derived(info?.active === true)

  async function refresh() {
    try {
      info = await invoke<RecordingInfo>('recording_status', { deviceId })
    } catch {
      // A PC that dropped mid-poll is already shown as offline in the grid; no second complaint.
      info = null
    }
  }

  $effect(() => {
    refresh()
    const timer = window.setInterval(refresh, 2000)
    return () => window.clearInterval(timer)
  })

  async function start() {
    busy = true
    try {
      const chosen = PRESETS[preset]
      info = await invoke<RecordingInfo>('start_recording', {
        deviceId,
        maxWidth: chosen.width,
        maxHeight: chosen.height,
        fps,
      })
      if (!info.active) onerror(info.problem || t('recFailed'))
      open = false
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function stop() {
    busy = true
    try {
      info = await invoke<RecordingInfo>('stop_recording', { deviceId })
      // The PC reports the rate it actually managed, which is often lower than asked.
      if (info.problem) onerror(info.problem)
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }
</script>

<span class="wrap">
  {#if recording}
    <button class="rec on" onclick={stop} disabled={busy}>
      <i class="blip"></i>
      {t('recStop', info?.frames ?? 0)}
    </button>
  {:else}
    <button class="rec" onclick={() => (open = !open)} disabled={busy}>{t('recStart')}</button>
  {/if}

  {#if open && !recording}
    <div class="menu" role="menu">
      <p class="label">{t('recSize')}</p>
      <div class="row">
        {#each PRESETS as item, index (item.label)}
          <button class="chip" class:active={preset === index} onclick={() => (preset = index)}>
            {item.label}
          </button>
        {/each}
      </div>
      <p class="label">{t('recRate')}</p>
      <div class="row">
        {#each RATES as rate (rate)}
          <button class="chip" class:active={fps === rate} onclick={() => (fps = rate)}>
            {rate} fps
          </button>
        {/each}
      </div>
      <p class="note">{t('recNote')}</p>
      <div class="row">
        <button class="primary" onclick={start} disabled={busy}>{t('recStart')}</button>
        <button onclick={() => (open = false)}>{t('cancel')}</button>
      </div>
    </div>
  {/if}
</span>

<style>
  .wrap {
    position: relative;
    display: inline-flex;
  }

  .rec.on {
    background: var(--danger);
    border-color: var(--danger);
    color: #1d0606;
    font-weight: 600;
  }

  .blip {
    display: inline-block;
    width: 7px;
    height: 7px;
    margin-right: 6px;
    border-radius: 50%;
    background: #1d0606;
  }

  .menu {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 20;
    width: 230px;
    padding: 8px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
  }

  .label {
    margin: 4px 2px 4px;
    color: var(--muted);
    font-size: 11.5px;
  }

  .note {
    margin: 6px 2px 8px;
    color: var(--muted);
    font-size: 11.5px;
    line-height: 1.45;
  }

  .row {
    display: flex;
    gap: 6px;
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
</style>
