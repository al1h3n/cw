<script lang="ts">
  import { t } from './i18n.svelte'
  import type { Device } from './types'

  let {
    device,
    onclose,
    onmonitor,
  }: { device: Device; onclose: () => void; onmonitor: (index: number) => void } = $props()

  function onkey(event: KeyboardEvent) {
    if (event.key === 'Escape') onclose()
  }
</script>

<svelte:window on:keydown={onkey} />

<!-- One screen, as large as the window allows: what the teacher opens to actually look at a PC. -->
<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="frame" role="dialog" aria-modal="true" aria-label={t('screenOf', device.device_id)}>
    <header>
      <span class="id">{device.device_id}</span>
      <span class="status"><i class="dot {device.status}"></i>{device.detail ?? device.status}</span>

      {#if device.monitors.length > 1}
        <span class="monitors" role="group" aria-label={t('monitors')}>
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
        </span>
      {/if}

      <button onclick={onclose}>{t('close')}</button>
    </header>
    <div class="screen">
      {#if device.screen}
        <img src={device.screen} alt={t('screenOf', device.device_id)} />
      {:else}
        <p class="hint">{t('waitingFirst')}</p>
      {/if}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(6, 9, 13, 0.82);
    padding: 18px;
  }

  .frame {
    display: grid;
    grid-template-rows: auto 1fr;
    width: min(1180px, 100%);
    max-height: 100%;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 14px;
    overflow: hidden;
  }

  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--line);
  }

  .id {
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
    font-size: 14px;
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
    font-size: 12px;
  }

  .monitors {
    display: inline-flex;
    gap: 6px;
    margin-left: auto;
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

  header > button:last-child {
    margin-left: auto;
  }

  .monitors ~ button:last-child {
    margin-left: 0;
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #48515e;
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

  .screen {
    display: grid;
    place-items: center;
    background: #05070a;
    min-height: 0;
    padding: 10px;
  }

  img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }

  .hint {
    color: var(--muted);
    font-size: 13px;
  }
</style>
