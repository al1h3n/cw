<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { t } from './i18n.svelte'
  import type { Device } from './types'

  /**
   * Present the teacher's screen (or one window) full-screen to the class, optionally locking the
   * students onto it. The source list and thumbnails come from the Rust side, like Zoom's picker.
   */
  let {
    devices,
    onclose,
    onerror,
  }: { devices: Device[]; onclose: () => void; onerror: (message: string) => void } = $props()

  type Source = { kind: string; id: number; title: string; primary: boolean; thumb: string }

  let sources = $state<Source[]>([])
  let loading = $state(true)
  let selected = $state<Source | null>(null)
  let locked = $state(false)
  let running = $state(false)
  let busy = $state(false)
  // What to do if the shared *window* is closed (or minimized too long): keep presenting the host
  // desktop, or stop the broadcast. Ignored for a whole-monitor source.
  let onClose = $state<'desktop' | 'stop'>('desktop')
  // Default: broadcast to every paired PC. Deselect to present to a subset.
  let targets = $state<Set<string>>(new Set(devices.map((d) => d.device_id)))

  // A broadcast started earlier keeps running even though this dialog was closed and reopened, so
  // reflect that on open: show it as live with a Stop button rather than a fresh Start.
  async function loadStatus() {
    try {
      const s = await invoke<{ running: boolean; targets: number }>('broadcast_status')
      running = s.running
    } catch {
      // No status is not fatal — the dialog just opens in its idle state.
    }
  }
  loadStatus()

  async function loadSources() {
    loading = true
    try {
      sources = await invoke<Source[]>('list_broadcast_sources')
    } catch (e) {
      onerror(String(e))
    } finally {
      loading = false
    }
  }
  loadSources()

  function pick(source: Source) {
    selected = source
  }

  function toggleTarget(id: string) {
    // Reassign so Svelte sees the change.
    const next = new Set(targets)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    targets = next
  }

  function toggleAll() {
    targets = targets.size === devices.length ? new Set() : new Set(devices.map((d) => d.device_id))
  }

  function label(source: Source): string {
    if (source.kind === 'monitor') {
      return source.primary ? t('broadcastDisplayMain', source.id + 1) : t('broadcastDisplay', source.id + 1)
    }
    return source.title || t('broadcastWindow')
  }

  async function start() {
    if (!selected || targets.size === 0 || busy) return
    busy = true
    try {
      await invoke('start_broadcast', {
        sourceKind: selected.kind,
        sourceId: selected.id,
        sourceTitle: label(selected),
        width: 1600,
        locked,
        onClose: selected.kind === 'window' ? onClose : 'desktop',
        targets: [...targets],
      })
      running = true
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function stop() {
    busy = true
    try {
      await invoke('stop_broadcast')
    } catch (e) {
      onerror(String(e))
    } finally {
      running = false
      busy = false
    }
  }

  // Closing the panel does NOT end the broadcast: a teacher can present and keep working in the
  // console, or close this picker entirely, while the class still sees the screen. The header shows a
  // persistent "presenting…" banner with its own Stop, and Stop here also ends it.
  function onkey(event: KeyboardEvent) {
    if (event.key === 'Escape') onclose()
  }
</script>

<svelte:window on:keydown={onkey} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('broadcastTitle')}>
    <h2>{t('broadcastTitle')}</h2>
    <p class="lead">{t('broadcastLead')}</p>

    <h3>{t('broadcastPickSource')}</h3>
    {#if loading}
      <p class="muted">{t('loading')}</p>
    {:else if sources.length === 0}
      <p class="muted">{t('broadcastNoSources')}</p>
    {:else}
      <div class="sources">
        {#each sources as source (source.kind + source.id)}
          <button
            class="source"
            class:sel={selected?.kind === source.kind && selected?.id === source.id}
            onclick={() => pick(source)}
            disabled={running}
          >
            <span class="thumb">
              {#if source.thumb}
                <img src={source.thumb} alt="" />
              {:else}
                <!-- No thumbnail: a minimized or hidden window cannot be captured by Windows, so we
                     draw a clean placeholder icon rather than a broken emoji box, and say why. -->
                <span class="noimg">
                  {#if source.kind === 'monitor'}
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">
                      <rect x="2" y="4" width="20" height="13" rx="2" />
                      <path d="M8 21h8M12 17v4" />
                    </svg>
                  {:else}
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">
                      <rect x="3" y="4" width="18" height="16" rx="2" />
                      <path d="M3 8h18" />
                    </svg>
                    <span class="minlabel">{t('broadcastMinimized')}</span>
                  {/if}
                </span>
              {/if}
            </span>
            <span class="cap">{label(source)}</span>
          </button>
        {/each}
      </div>
    {/if}

    <h3>{t('broadcastPickTargets')}</h3>
    <div class="targets">
      <label class="all">
        <input
          type="checkbox"
          checked={targets.size === devices.length && devices.length > 0}
          onchange={toggleAll}
          disabled={running}
        />
        {t('broadcastAll', targets.size, devices.length)}
      </label>
      <div class="devlist">
        {#each devices as device (device.device_id)}
          <label>
            <input
              type="checkbox"
              checked={targets.has(device.device_id)}
              onchange={() => toggleTarget(device.device_id)}
              disabled={running}
            />
            <span class="dname">{device.name || device.device_id}</span>
          </label>
        {/each}
      </div>
    </div>

    <label class="lock">
      <input type="checkbox" bind:checked={locked} disabled={running} />
      <span>
        <strong>{t('broadcastLock')}</strong>
        <span class="hint">{t('broadcastLockHint')}</span>
      </span>
    </label>

    {#if selected?.kind === 'window'}
      <label class="onclose">
        <span class="lbl">{t('broadcastOnClose')}</span>
        <select bind:value={onClose} disabled={running}>
          <option value="desktop">{t('broadcastOnCloseDesktop')}</option>
          <option value="stop">{t('broadcastOnCloseStop')}</option>
        </select>
      </label>
    {/if}

    <footer>
      <button onclick={loadSources} disabled={running || busy}>{t('broadcastRefresh')}</button>
      <span class="spacer"></span>
      {#if running}
        <span class="live" aria-live="polite">● {t('broadcastLive', targets.size)}</span>
        <button class="danger" onclick={stop} disabled={busy}>{t('broadcastStop')}</button>
      {:else}
        <button
          class="primary"
          onclick={start}
          disabled={!selected || targets.size === 0 || busy}
        >
          {t('broadcastStart')}
        </button>
      {/if}
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
    width: min(760px, 96vw);
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

  .sources {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 10px;
  }

  .source {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px;
    background: var(--bg);
    border: 2px solid var(--line);
    border-radius: 10px;
    cursor: pointer;
    text-align: left;
  }

  .source.sel {
    border-color: var(--accent);
  }

  .thumb {
    display: grid;
    place-items: center;
    aspect-ratio: 16 / 9;
    overflow: hidden;
    background: #05070a;
    border-radius: 6px;
  }

  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .noimg {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    color: var(--muted);
  }

  .noimg svg {
    width: 30px;
    height: 30px;
    opacity: 0.7;
  }

  .minlabel {
    font-size: 10px;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    opacity: 0.75;
  }

  .cap {
    font-size: 12px;
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
    max-height: 140px;
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

  .lock {
    align-items: flex-start;
    margin-top: 16px;
    padding: 10px 12px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 10px;
  }

  .lock span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .lock .hint {
    color: var(--muted);
    font-size: 12px;
  }

  .onclose {
    align-items: center;
    gap: 10px;
    margin-top: 10px;
  }

  .onclose .lbl {
    color: var(--muted);
    font-size: 12.5px;
  }

  .onclose select {
    padding: 5px 8px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font: inherit;
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

  .spacer {
    flex: 1;
  }

  .live {
    color: var(--live);
    font-size: 12.5px;
    font-weight: 600;
  }

  .danger:hover {
    border-color: var(--danger);
    color: var(--danger);
  }

  .muted {
    color: var(--muted);
    font-size: 12.5px;
  }
</style>
