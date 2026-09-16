<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { t } from './i18n.svelte'
  import type { RecordingInfo } from './types'

  /** Start/stop recording for one PC, with the codec, size, rate and quality the teacher picks. */
  let { deviceId, onerror }: { deviceId: string; onerror: (message: string) => void } = $props()

  const SIZES = [
    { label: '720p', width: 1280, height: 720 },
    { label: '1080p', width: 1920, height: 1080 },
    { label: '1440p', width: 2560, height: 1440 },
    { label: '4K', width: 3840, height: 2160 },
  ]

  let info = $state<RecordingInfo | null>(null)
  let open = $state(false)
  let busy = $state(false)

  // Encoder choices. Codec/preset/scaler are the lowercase tags the Rust command maps to enums.
  // These apply when ffmpeg.exe sits next to the agent; without it the PC records MJPEG and only
  // size/rate matter (the note in the menu says so).
  let width = $state(1920)
  let height = $state(1080)
  let fps = $state(15)
  let codec = $state('h264')
  let preset = $state('medium')
  let quality = $state(23)
  let bframes = $state(8)
  let scaler = $state('lanczos')

  const recording = $derived(info?.active === true)

  async function refresh() {
    try {
      info = await invoke<RecordingInfo>('recording_status', { deviceId })
    } catch {
      info = null
    }
  }

  $effect(() => {
    refresh()
    const timer = window.setInterval(refresh, 2000)
    return () => window.clearInterval(timer)
  })

  function size(label: string) {
    const found = SIZES.find((s) => s.label === label)
    if (found) {
      width = found.width
      height = found.height
    }
  }

  async function start() {
    busy = true
    try {
      info = await invoke<RecordingInfo>('start_recording', {
        deviceId,
        maxWidth: Math.max(160, Math.min(3840, Math.round(width))),
        maxHeight: Math.max(120, Math.min(2160, Math.round(height))),
        fps: Math.max(1, Math.min(30, Math.round(fps))),
        codec,
        preset,
        quality: Math.max(0, Math.min(51, Math.round(quality))),
        bframes: Math.max(0, Math.min(16, Math.round(bframes))),
        scaler,
        twoPass: false,
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
      <label>
        {t('recCodec')}
        <select bind:value={codec}>
          <option value="h264">H.264</option>
          <option value="h265">H.265</option>
          <option value="av1">AV1</option>
        </select>
      </label>

      <p class="label">{t('recSize')}</p>
      <div class="row">
        {#each SIZES as item (item.label)}
          <button
            class="chip"
            class:active={width === item.width && height === item.height}
            onclick={() => size(item.label)}
          >
            {item.label}
          </button>
        {/each}
      </div>
      <div class="row">
        <input type="number" min="160" max="3840" bind:value={width} aria-label="width" />
        <span class="x">×</span>
        <input type="number" min="120" max="2160" bind:value={height} aria-label="height" />
        <input class="fps" type="number" min="1" max="30" bind:value={fps} aria-label="fps" />
        <span class="unit">fps</span>
      </div>

      <label>
        {t('recPreset')}
        <select bind:value={preset}>
          <option value="ultrafast">ultrafast</option>
          <option value="veryfast">veryfast</option>
          <option value="fast">fast</option>
          <option value="medium">medium</option>
          <option value="slow">slow</option>
          <option value="veryslow">veryslow</option>
        </select>
      </label>

      <label>
        {t('recQuality')} <span class="num">{quality}</span>
        <input type="range" min="14" max="34" bind:value={quality} />
      </label>

      <div class="two">
        <label>
          {t('recScaler')}
          <select bind:value={scaler}>
            <option value="lanczos">lanczos</option>
            <option value="bicubic">bicubic</option>
            <option value="bilinear">bilinear</option>
            <option value="neighbor">neighbor</option>
          </select>
        </label>
        <label>
          {t('recBframes')}
          <input type="number" min="0" max="16" bind:value={bframes} />
        </label>
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
    width: 264px;
    max-height: 78vh;
    overflow: auto;
    padding: 10px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
  }

  label {
    display: block;
    margin: 8px 2px 4px;
    color: var(--muted);
    font-size: 11.5px;
  }

  .label {
    margin: 8px 2px 4px;
    color: var(--muted);
    font-size: 11.5px;
  }

  select,
  input[type='number'],
  input[type='range'] {
    width: 100%;
    margin-top: 3px;
    padding: 5px 6px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font-size: 12.5px;
  }

  input[type='range'] {
    padding: 0;
  }

  .note {
    margin: 8px 2px;
    color: var(--muted);
    font-size: 11px;
    line-height: 1.45;
  }

  .row {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-bottom: 4px;
  }

  .row input {
    width: auto;
    flex: 1;
    margin: 0;
  }

  .row .fps {
    max-width: 3.5em;
    flex: 0 0 auto;
  }

  .x,
  .unit {
    color: var(--muted);
    font-size: 12px;
  }

  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .two label {
    margin-top: 4px;
  }

  .num {
    color: var(--text);
    font-weight: 600;
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

  .primary {
    flex: 1;
  }
</style>
