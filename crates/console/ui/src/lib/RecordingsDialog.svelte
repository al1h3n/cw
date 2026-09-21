<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { onDestroy } from 'svelte'
  import { t } from './i18n.svelte'
  import { toasts } from './toast-store.svelte'

  /**
   * Host-wide recording control: see which PCs are recording right now, start or stop recording on
   * the whole class at once, and pull every recording down into a single .zip on this PC.
   */
  let { onclose, onerror }: { onclose: () => void; onerror: (message: string) => void } = $props()

  type Rec = { file: string; bytes: number }
  type Row = {
    device_id: string
    name: string | null
    active: boolean
    frames: number
    connected: boolean
    recordings: Rec[]
  }

  let rows = $state<Row[]>([])
  let busy = $state(false)
  let zipping = $state(false)

  // Bulk record options (kept simple; per-PC fine control still lives in the focused view).
  let res = $state('1280x720')
  let fps = $state(15)
  let codec = $state('h264')
  let twoPass = $state(false)

  const activeCount = $derived(rows.filter((r) => r.active).length)
  const totalRecordings = $derived(rows.reduce((n, r) => n + r.recordings.length, 0))

  async function refresh() {
    try {
      rows = await invoke<Row[]>('recording_overview')
    } catch (e) {
      onerror(String(e))
    }
  }
  refresh()
  // Keep the "recording now" dots live.
  const timer = setInterval(refresh, 2000)
  onDestroy(() => clearInterval(timer))

  async function recordAll() {
    if (busy) return
    busy = true
    try {
      const [w, h] = res.split('x').map(Number)
      const result = await invoke<{ ok: number; failed: number }>('record_all', {
        maxWidth: w,
        maxHeight: h,
        fps: Math.max(1, Math.min(60, fps)),
        codec,
        preset: 'medium',
        quality: 23,
        bframes: 8,
        scaler: 'lanczos',
        twoPass,
      })
      toasts.push(t('recAllStarted', result.ok, result.failed), result.failed ? 'error' : 'ok')
      await refresh()
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function stopAll() {
    if (busy) return
    busy = true
    try {
      const result = await invoke<{ ok: number; failed: number }>('stop_all_recording')
      toasts.push(t('recAllStopped', result.ok), 'ok')
      await refresh()
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function downloadZip() {
    if (zipping) return
    zipping = true
    try {
      const path = await invoke<string>('download_all_recordings_zip')
      toasts.push(t('recZipSaved', path), 'ok')
    } catch (e) {
      onerror(String(e))
    } finally {
      zipping = false
    }
  }

  async function downloadOne(deviceId: string, file: string) {
    try {
      const path = await invoke<string>('download_recording', { deviceId, file })
      toasts.push(t('recSavedTo', path), 'ok')
    } catch (e) {
      onerror(String(e))
    }
  }

  function sizeText(bytes: number): string {
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
    if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`
    return `${bytes} B`
  }

  function onkey(event: KeyboardEvent) {
    if (event.key === 'Escape') onclose()
  }
</script>

<svelte:window on:keydown={onkey} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('recordingsTitle')}>
    <h2>{t('recordingsTitle')}</h2>
    <p class="lead">{t('recordingsLead', activeCount, totalRecordings)}</p>

    <div class="toolbar">
      <select bind:value={res} aria-label={t('resolution')}>
        <option value="1280x720">720p</option>
        <option value="1920x1080">1080p</option>
        <option value="2560x1440">1440p</option>
      </select>
      <input type="number" min="1" max="60" bind:value={fps} aria-label={t('fpsLabel')} />
      <select bind:value={codec} aria-label={t('recCodec')}>
        <option value="h264">H.264</option>
        <option value="h265">H.265</option>
        <option value="av1">AV1</option>
      </select>
      <label class="tp"><input type="checkbox" bind:checked={twoPass} />{t('recTwoPass')}</label>
      <button class="primary" onclick={recordAll} disabled={busy}>{t('recAll')}</button>
      <button class="danger" onclick={stopAll} disabled={busy}>{t('recStopAll')}</button>
      <span class="spacer"></span>
      <button onclick={downloadZip} disabled={zipping || totalRecordings === 0}>
        {zipping ? t('recZipping') : t('recDownloadZip')}
      </button>
    </div>

    <div class="rows">
      {#each rows as row (row.device_id)}
        <div class="row" class:off={!row.connected}>
          <div class="head">
            {#if row.active}
              <span class="rec" title={t('recActive')}>●</span>
            {:else}
              <span class="dot" class:on={row.connected}></span>
            {/if}
            <span class="who">{row.name || row.device_id}</span>
            {#if row.active}
              <span class="frames">{t('recFrames', row.frames)}</span>
            {:else if !row.connected}
              <span class="muted">{t('recNotWatched')}</span>
            {/if}
          </div>
          {#if row.recordings.length > 0}
            <ul>
              {#each row.recordings as rec (rec.file)}
                <li>
                  <span class="file">{rec.file}</span>
                  <span class="size">{sizeText(rec.bytes)}</span>
                  <button onclick={() => downloadOne(row.device_id, rec.file)}>
                    {t('recDownload')}
                  </button>
                </li>
              {/each}
            </ul>
          {:else if row.connected}
            <p class="muted small">{t('recNone')}</p>
          {/if}
        </div>
      {/each}
      {#if rows.length === 0}
        <p class="muted">{t('loading')}</p>
      {/if}
    </div>

    <footer>
      <span class="spacer"></span>
      <button onclick={onclose}>{t('close')}</button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    background: rgba(6, 9, 13, 0.82);
    padding: 18px;
  }

  .dialog {
    display: flex;
    flex-direction: column;
    width: min(720px, 96vw);
    max-height: 92vh;
    padding: 22px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 14px;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 17px;
  }

  .lead {
    margin: 0 0 12px;
    color: var(--muted);
    font-size: 13px;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding-bottom: 12px;
    margin-bottom: 12px;
    border-bottom: 1px solid var(--line);
  }

  .toolbar select,
  .toolbar input[type='number'] {
    padding: 5px 7px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--text);
    font-size: 12px;
  }

  .toolbar input[type='number'] {
    width: 3.4em;
  }

  .tp {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    color: var(--muted);
  }

  .spacer {
    flex: 1;
  }

  .rows {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .row {
    padding: 8px 10px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 10px;
  }

  .row.off {
    opacity: 0.6;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .who {
    font-size: 13px;
    font-weight: 600;
  }

  .frames {
    font-size: 11.5px;
    color: var(--muted);
  }

  .rec {
    color: var(--danger);
    animation: pulse 1.4s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .rec {
      animation: none;
    }
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #48515e;
  }

  .dot.on {
    background: var(--live);
  }

  ul {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    display: grid;
    gap: 4px;
  }

  li {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }

  .file {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
  }

  .size {
    color: var(--muted);
    flex-shrink: 0;
  }

  .muted {
    color: var(--muted);
    font-size: 12.5px;
  }

  .small {
    margin: 6px 0 0;
    font-size: 11.5px;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 12px;
    margin-top: 12px;
    border-top: 1px solid var(--line);
  }

  .danger:hover {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
