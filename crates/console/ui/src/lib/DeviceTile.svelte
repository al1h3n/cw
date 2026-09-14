<script lang="ts">
  import { t } from './i18n'
  import type { Device } from './types'

  let { device, watching, onopen }: { device: Device; watching: boolean; onopen: () => void } = $props()

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

<!-- The screen is the tile. Everything else sits quietly on top of it. -->
<button class="tile" onclick={onopen} aria-label={t('screenOf', device.device_id)}>
  <div class="screen">
    {#if device.screen}
      <img src={device.screen} alt={t('screenOf', device.device_id)} />
    {:else}
      <p class="hint">{watching ? t('waitingFirst') : t('notWatching')}</p>
    {/if}
  </div>

  <div class="bar">
    <span class="id">{device.device_id}</span>
    <span class="status">
      <i class="dot {device.status}"></i>
      {device.detail ?? label}
    </span>
  </div>
</button>

<style>
  .tile {
    display: block;
    width: 100%;
    padding: 0;
    overflow: hidden;
    text-align: left;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
  }

  .tile:hover {
    border-color: #42506a;
    background: var(--panel);
  }

  .screen {
    position: relative;
    aspect-ratio: 16 / 9;
    display: grid;
    place-items: center;
    background: #0a0d11;
    border-bottom: 1px solid var(--line);
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
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
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
</style>
