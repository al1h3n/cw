<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { t } from './i18n.svelte'
  import type { Device } from './types'

  /**
   * Push a chosen desktop wallpaper to one, several, or every connected student PC. The teacher
   * picks an image file here; its bytes are sent to the Agent, which sets it as the desktop
   * background. Only connected PCs can be changed, so offline ones are shown but not targetable.
   */
  let {
    devices,
    onclose,
    onerror,
  }: { devices: Device[]; onclose: () => void; onerror: (message: string) => void } = $props()

  let fileName = $state('')
  let preview = $state('')
  let bytes = $state<Uint8Array | null>(null)
  let busy = $state(false)
  // How the image is laid out on each desktop (crop/fit/stretch/centre/tile).
  let fit = $state<'fill' | 'fit' | 'stretch' | 'center' | 'tile'>('fill')
  // Default: every paired PC. Deselect to change a subset.
  let targets = $state<Set<string>>(new Set(devices.map((d) => d.device_id)))

  async function pickFile(event: Event) {
    const input = event.target as HTMLInputElement
    const file = input.files?.[0]
    if (!file) return
    try {
      const buffer = await file.arrayBuffer()
      bytes = new Uint8Array(buffer)
      fileName = file.name
      // A data URL just for the on-screen preview; the raw bytes are what get sent.
      preview = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader()
        reader.onload = () => resolve(String(reader.result))
        reader.onerror = () => reject(reader.error)
        reader.readAsDataURL(file)
      })
    } catch (e) {
      onerror(String(e))
    }
  }

  function toggleTarget(id: string) {
    const next = new Set(targets)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    targets = next
  }

  function toggleAll() {
    targets = targets.size === devices.length ? new Set() : new Set(devices.map((d) => d.device_id))
  }

  async function apply() {
    if (!bytes || targets.size === 0 || busy) return
    busy = true
    try {
      const result = await invoke<{ ok: number; failed: number }>('set_wallpaper', {
        targets: [...targets],
        image: Array.from(bytes),
        fit,
      })
      if (result.failed > 0 && result.ok === 0) {
        onerror(t('wallpaperAllFailed', result.failed))
      }
      onclose()
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  function onkey(event: KeyboardEvent) {
    if (event.key === 'Escape') onclose()
  }
</script>

<svelte:window on:keydown={onkey} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('wallpaperTitle')}>
    <h2>{t('wallpaperTitle')}</h2>
    <p class="lead">{t('wallpaperLead')}</p>

    <h3>{t('wallpaperPickImage')}</h3>
    <div class="pick">
      <label class="file">
        <input type="file" accept="image/png,image/jpeg,image/bmp,image/gif" onchange={pickFile} />
        <span>{t('wallpaperChoose')}</span>
      </label>
      <div class="preview">
        {#if preview}
          <img src={preview} alt="" />
        {:else}
          <span class="noimg">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
              <rect x="3" y="4" width="18" height="16" rx="2" />
              <circle cx="9" cy="9" r="1.6" />
              <path d="m4 17 5-4 4 3 3-2 4 3" />
            </svg>
          </span>
        {/if}
      </div>
      <span class="fname">{fileName || t('wallpaperNoFile')}</span>
    </div>

    <label class="fitrow">
      <span class="fitlbl">{t('wallpaperFit')}</span>
      <select bind:value={fit}>
        <option value="fill">{t('fitFill')}</option>
        <option value="fit">{t('fitFit')}</option>
        <option value="stretch">{t('fitStretch')}</option>
        <option value="center">{t('fitCenter')}</option>
        <option value="tile">{t('fitTile')}</option>
      </select>
    </label>

    <h3>{t('wallpaperPickTargets')}</h3>
    <div class="targets">
      <label class="all">
        <input
          type="checkbox"
          checked={targets.size === devices.length && devices.length > 0}
          onchange={toggleAll}
        />
        {t('wallpaperAll', targets.size, devices.length)}
      </label>
      <div class="devlist">
        {#each devices as device (device.device_id)}
          <label>
            <input
              type="checkbox"
              checked={targets.has(device.device_id)}
              onchange={() => toggleTarget(device.device_id)}
            />
            <span class="dname">{device.name || device.device_id}</span>
          </label>
        {/each}
      </div>
    </div>

    <footer>
      <span class="spacer"></span>
      <button class="primary" onclick={apply} disabled={!bytes || targets.size === 0 || busy}>
        {busy ? t('wallpaperApplying') : t('wallpaperApply')}
      </button>
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
    width: min(620px, 96vw);
    max-height: 92vh;
    overflow: auto;
    padding: 22px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 14px;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 17px;
  }

  h3 {
    margin: 16px 0 8px;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }

  .lead {
    margin: 0;
    color: var(--muted);
    font-size: 13px;
  }

  .pick {
    display: grid;
    grid-template-columns: auto 1fr;
    grid-template-rows: auto auto;
    gap: 8px 12px;
    align-items: center;
  }

  .file {
    position: relative;
    grid-row: 1 / 3;
    display: grid;
    place-items: center;
    width: 160px;
    aspect-ratio: 16 / 9;
    background: var(--bg);
    border: 2px dashed var(--line);
    border-radius: 10px;
    cursor: pointer;
    font-size: 13px;
    text-align: center;
  }

  .file input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }

  .preview {
    display: grid;
    place-items: center;
    aspect-ratio: 16 / 9;
    max-width: 220px;
    overflow: hidden;
    background: #05070a;
    border-radius: 8px;
  }

  .preview img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .noimg {
    display: grid;
    place-items: center;
    color: var(--muted);
    opacity: 0.5;
  }

  .noimg svg {
    width: 30px;
    height: 30px;
  }

  .fitrow {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 12px;
  }

  .fitlbl {
    color: var(--muted);
    font-size: 12.5px;
  }

  .fitrow select {
    padding: 5px 8px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
  }

  .fname {
    font-size: 12px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .targets {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .all {
    font-weight: 600;
    font-size: 13px;
  }

  .devlist {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 4px 12px;
    max-height: 160px;
    overflow: auto;
    padding: 6px 8px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
  }

  label {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }

  .dname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 16px;
    margin-top: 16px;
    border-top: 1px solid var(--line);
    flex-wrap: wrap;
  }

  .spacer {
    flex: 1;
  }
</style>
