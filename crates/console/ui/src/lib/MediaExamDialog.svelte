<script lang="ts">
  import { invoke } from './bridge'
  import { t } from './i18n.svelte'
  import type { Device } from './types'

  /**
   * A listening exam: preload one audio file to the chosen PCs, then start it on all of them together
   * (feature 14). Playback has no controls and the file is deleted after it plays once; ticking "lock"
   * also locks each screen with the message while it plays. Only connected PCs can take part.
   *
   * Synchronised without trusting the student's clock: every PC starts a fixed delay after it *receives*
   * the play command, so a changed timezone cannot desync it.
   */
  let {
    devices,
    onclose,
    onerror,
  }: { devices: Device[]; onclose: () => void; onerror: (message: string) => void } = $props()

  const live = $derived(devices.filter((d) => d.status === 'live'))

  let fileName = $state('')
  let bytes = $state<Uint8Array | null>(null)
  let message = $state('')
  let lock = $state(true)
  let startSeconds = $state(3)
  let busy = $state(false)
  let status = $state('')
  let targets = $state<Set<string>>(new Set())

  // Default to every connected PC once the list is known.
  $effect(() => {
    if (targets.size === 0 && live.length > 0) targets = new Set(live.map((d) => d.device_id))
  })

  function pickFile(event: Event) {
    const input = event.target as HTMLInputElement
    const file = input.files?.[0]
    if (!file) return
    file.arrayBuffer().then((buf) => {
      bytes = new Uint8Array(buf)
      fileName = file.name
    })
  }

  function toggleTarget(id: string) {
    const next = new Set(targets)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    targets = next
  }

  async function play() {
    if (!bytes || targets.size === 0 || busy) return
    const ids = [...targets].filter((id) => live.some((d) => d.device_id === id))
    busy = true
    status = t('mediaPreloading')
    try {
      // Preload to everyone first, so the actual start can be near-simultaneous.
      for (const id of ids) {
        await invoke('preload_media', { deviceId: id, name: fileName, data: Array.from(bytes) })
      }
      // Then start them together: each PC begins the same delay after it receives this, so the small
      // spread of sending N commands is absorbed rather than desyncing playback.
      const startInMs = Math.max(0, Math.round(startSeconds * 1000))
      status = t('mediaStarting')
      for (const id of ids) {
        await invoke('play_media', { deviceId: id, startInMs, message, lock })
      }
      status = t('mediaPlaying', ids.length)
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function stop() {
    if (busy) return
    busy = true
    try {
      for (const id of [...targets]) {
        try {
          await invoke('stop_media', { deviceId: id })
        } catch {
          /* a PC that went offline is already stopped */
        }
      }
      status = t('mediaStopped')
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
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('mediaTitle')}>
    <h2>{t('mediaTitle')}</h2>
    <p class="lead">{t('mediaLead')}</p>

    <label class="file">
      <input type="file" accept="audio/*" onchange={pickFile} />
      <span>{fileName || t('mediaPickFile')}</span>
    </label>

    <div class="opts">
      <label class="opt">
        <span>{t('mediaStartIn')}</span>
        <input type="number" min="0" max="60" bind:value={startSeconds} />
      </label>
      <label class="opt lock">
        <input type="checkbox" bind:checked={lock} />
        {t('mediaLock')}
      </label>
    </div>

    <label class="msg">
      <span>{t('mediaMessage')}</span>
      <input type="text" bind:value={message} placeholder={t('mediaMessagePlaceholder')} />
    </label>

    <h3>{t('mediaPickTargets')}</h3>
    {#if live.length === 0}
      <p class="hint">{t('mediaNoTargets')}</p>
    {:else}
      <div class="devlist">
        {#each live as device (device.device_id)}
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
    {/if}

    <footer>
      <span class="status">{status}</span>
      <span class="spacer"></span>
      <button onclick={stop} disabled={busy}>{t('mediaStop')}</button>
      <button class="primary" onclick={play} disabled={!bytes || targets.size === 0 || busy}>
        {t('mediaPlay')}
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
    width: min(560px, 96vw);
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
    margin: 0 0 14px;
    color: var(--muted);
    font-size: 13px;
  }
  .file {
    position: relative;
    display: grid;
    place-items: center;
    padding: 14px;
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
  .opts {
    display: flex;
    align-items: center;
    gap: 18px;
    margin-top: 12px;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--muted);
  }
  .opt input[type='number'] {
    width: 64px;
    padding: 5px 8px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font: inherit;
  }
  .msg {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 12px;
    font-size: 12px;
    color: var(--muted);
  }
  .msg input {
    padding: 7px 9px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .devlist {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: 4px 12px;
    max-height: 200px;
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
  .hint {
    color: var(--muted);
    font-size: 12.5px;
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
  .status {
    font-size: 12px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .spacer {
    flex: 1;
  }
</style>
