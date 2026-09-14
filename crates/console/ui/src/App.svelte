<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { onDestroy, onMount } from 'svelte'
  import DeviceTile from './lib/DeviceTile.svelte'
  import PairDialog from './lib/PairDialog.svelte'
  import Focused from './lib/Focused.svelte'
  import LanguagePicker from './lib/LanguagePicker.svelte'
  import QualityPicker from './lib/QualityPicker.svelte'
  import { i18n, t } from './lib/i18n.svelte'
  import type { ConsoleInfo, Device } from './lib/types'

  let info = $state<ConsoleInfo | null>(null)
  let devices = $state<Device[]>([])
  let watching = $state(false)
  let pairing = $state(false)
  let focused = $state<string | null>(null)
  let error = $state<string | null>(null)
  let loaded = $state(false)
  let timer: number | undefined

  const live = $derived(devices.filter((d) => d.status === 'live').length)
  const focusedDevice = $derived(devices.find((d) => d.device_id === focused) ?? null)

  async function refresh() {
    try {
      devices = await invoke<Device[]>('devices')
      loaded = true
    } catch (e) {
      error = String(e)
    }
  }

  /** Opening a screen asks the backend to refresh it faster and larger. */
  async function open(deviceId: string | null) {
    focused = deviceId
    try {
      await invoke('set_focused', { deviceId })
    } catch (e) {
      error = String(e)
    }
  }

  async function chooseMonitor(deviceId: string, index: number) {
    try {
      await invoke('set_monitor', { deviceId, monitor: index })
      await refresh()
    } catch (e) {
      error = String(e)
    }
  }

  async function toggleWatching() {
    error = null
    try {
      if (watching) {
        await invoke('stop_watching')
        watching = false
      } else {
        await invoke('start_watching')
        watching = true
      }
    } catch (e) {
      error = String(e)
    }
  }

  onMount(async () => {
    try {
      // Strings first, so nothing renders in the wrong language.
      await i18n.load()
      info = await invoke<ConsoleInfo>('console_info')
    } catch (e) {
      error = String(e)
    }
    await refresh()
    // One poll drives the whole grid; the agents only capture while watching is on.
    timer = window.setInterval(refresh, 1000)
  })

  onDestroy(() => {
    if (timer) window.clearInterval(timer)
  })
</script>

<!-- Nothing renders until the strings are in, so raw keys never flash on screen. -->
{#if !i18n.ready}
  <div class="boot"></div>
{:else}
<div class="shell">
  <header>
    <div class="title">
      <h1>{t('room')}</h1>
      <p class="sub">
        {#if loaded}
          {devices.length === 0 ? t('noDevices') : t('deviceCount', devices.length, live)}
        {:else}
          {t('loading')}
        {/if}
      </p>
    </div>

    <div class="actions">
      <button
        class:primary={!watching}
        onclick={toggleWatching}
        disabled={devices.length === 0}
        title={devices.length === 0 ? t('addFirst') : ''}
      >
        {watching ? t('stopWatching') : t('startWatching')}
      </button>
      <button onclick={() => (pairing = true)}>{t('addPc')}</button>
    </div>
  </header>

  {#if error}
    <div class="banner" role="alert">
      <span>{error}</span>
      <button onclick={() => (error = null)}>{t('dismiss')}</button>
    </div>
  {/if}

  <main>
    {#if !loaded}
      <p class="placeholder">{t('loading')}</p>
    {:else if devices.length === 0}
      <div class="empty">
        <h2>{t('emptyTitle')}</h2>
        <p>{t('emptyBody')}</p>
        <button class="primary" onclick={() => (pairing = true)}>{t('addPc')}</button>
      </div>
    {:else}
      <div class="grid">
        {#each devices as device (device.device_id)}
          <DeviceTile
            {device}
            {watching}
            onopen={() => open(device.device_id)}
            onmonitor={(index) => chooseMonitor(device.device_id, index)}
          />
        {/each}
      </div>
    {/if}
  </main>

  <footer>
    {#if info}
      <span>{t('thisConsole')} <code>{info.device_id}</code></span>
    {/if}
    <span class="right">
      <QualityPicker />
      <span class="dot-label">
        <i class="dot" class:live={watching}></i>
        {watching ? t('capturing') : t('notCapturing')}
      </span>
      <LanguagePicker />
    </span>
  </footer>
</div>
{/if}

{#if pairing}
  <PairDialog
    onclose={() => {
      pairing = false
      refresh()
    }}
  />
{/if}

{#if focusedDevice}
  <Focused
    device={focusedDevice}
    onclose={() => open(null)}
    onmonitor={(index) => chooseMonitor(focusedDevice.device_id, index)}
  />
{/if}

<style>
  .shell {
    display: grid;
    grid-template-rows: auto auto 1fr auto;
    height: 100%;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 16px 20px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
  }

  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }

  .sub {
    margin: 2px 0 0;
    color: var(--muted);
    font-size: 13px;
  }

  .actions {
    display: flex;
    gap: 10px;
    flex-shrink: 0;
  }

  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 20px;
    background: #3a1f1f;
    border-bottom: 1px solid #5a2b2b;
    color: #ffd7d7;
    font-size: 13px;
  }

  main {
    overflow: auto;
    padding: 18px 20px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 14px;
  }

  .empty,
  .placeholder {
    max-width: 420px;
    margin: 12vh auto 0;
    text-align: center;
    color: var(--muted);
  }

  .empty h2 {
    margin: 0 0 6px;
    color: var(--text);
    font-size: 17px;
  }

  .empty p {
    margin: 0 0 18px;
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 9px 20px;
    border-top: 1px solid var(--line);
    background: var(--panel);
    color: var(--muted);
    font-size: 12px;
  }

  .boot {
    height: 100%;
    background: var(--bg);
  }

  .right {
    display: inline-flex;
    align-items: center;
    gap: 14px;
  }

  .dot-label {
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #48515e;
  }

  .dot.live {
    background: var(--live);
    box-shadow: 0 0 0 3px rgba(62, 207, 142, 0.18);
  }
</style>
