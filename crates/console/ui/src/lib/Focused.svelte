<script lang="ts">
  import { t } from './i18n.svelte'
  import type { Device } from './types'

  let { device, onclose }: { device: Device; onclose: () => void } = $props()

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

  header button {
    margin-left: auto;
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
