<script lang="ts">
  import { t } from './i18n.svelte'
  import type { Device } from './types'
  import ActionResult from './ActionResult.svelte'

  let {
    device,
    watching,
    onopen,
    onmonitor,
    onwake,
  }: {
    device: Device
    watching: boolean
    onopen: () => void
    onmonitor: (index: number) => void
    onwake: () => void
  } = $props()

  const label = $derived(
    device.status === 'live'
      ? t('statusLive')
      : device.status === 'connecting'
        ? t('statusConnecting')
        : device.status === 'offline'
          ? t('statusOffline')
          : t('statusIdle'),
  )
</script>

<div class="card">
  <!-- The screen is the tile. Everything else sits quietly under it. -->
  <button class="screen" onclick={onopen} aria-label={t('screenOf', device.device_id)}>
    {#if device.screen}
      <img src={device.screen} alt={t('screenOf', device.device_id)} />
    {:else}
      <p class="hint">{watching ? t('waitingFirst') : t('notWatching')}</p>
    {/if}
  </button>

  <div class="bar">
    <span class="id">{device.device_id}</span>
    <span class="status">
      <ActionResult report={device.last_action} />
      {#if device.status === 'offline' && device.macs.length > 0}
        <button class="wake" onclick={(e) => (e.stopPropagation(), onwake())} title={t('wakeHint')}>
          {t('wake')}
        </button>
      {/if}
      <i class="dot {device.status}"></i>
      {device.detail ?? label}
    </span>
  </div>

  {#if device.monitors.length > 1}
    <!-- Only shown when the PC really has more than one screen, so single-monitor tiles stay clean. -->
    <div class="monitors" role="group" aria-label={t('monitors')}>
      {#each device.monitors as monitor (monitor.index)}
        <button
          class="chip"
          class:active={monitor.index === device.monitor}
          onclick={() => onmonitor(monitor.index)}
          title={`${monitor.width}×${monitor.height}`}
        >
          {monitor.primary ? t('monitorMain') : t('monitorNumber', monitor.index + 1)}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .card {
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .card:hover {
    border-color: #42506a;
  }

  .screen {
    display: grid;
    place-items: center;
    width: 100%;
    padding: 0;
    aspect-ratio: 16 / 9;
    background: #0a0d11;
    border: 0;
    border-bottom: 1px solid var(--line);
    border-radius: 0;
    cursor: pointer;
  }

  .screen:hover {
    background: #0a0d11;
  }

  img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }

  .hint {
    margin: 0;
    padding: 0 14px;
    color: var(--muted);
    font-size: 12.5px;
    text-align: center;
  }

  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 10px;
  }

  .id {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 13px;
    letter-spacing: 0.4px;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
    font-size: 12px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .wake {
    padding: 2px 8px;
    font-size: 11px;
    border-radius: 999px;
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #48515e;
    flex-shrink: 0;
  }

  .dot.live {
    background: var(--live);
  }

  .dot.connecting {
    background: var(--warn);
  }

  .dot.offline {
    background: var(--danger);
  }

  .monitors {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 0 10px 10px;
  }

  .chip {
    padding: 4px 10px;
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
