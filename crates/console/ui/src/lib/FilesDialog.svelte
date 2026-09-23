<script lang="ts">
  import { invoke } from './bridge'
  import { t } from './i18n.svelte'
  import type { Device } from './types'

  /**
   * Send files to a student PC and download files they made, inside a shared **workspace** folder.
   * Transfer is confined to that folder on the Agent (path traversal refused), so this browses,
   * uploads to, and downloads from one directory tree — not the whole PC. Only connected PCs answer.
   */
  type FileEntry = { name: string; is_dir: boolean; bytes: number }

  let {
    devices,
    onclose,
    onerror,
  }: { devices: Device[]; onclose: () => void; onerror: (message: string) => void } = $props()

  const live = $derived(devices.filter((d) => d.status === 'live'))
  let deviceId = $state<string>('')
  // Current directory as workspace-relative path segments (empty = the workspace root).
  let segments = $state<string[]>([])
  const dir = $derived(segments.join('/'))
  let entries = $state<FileEntry[]>([])
  let loading = $state(false)
  let busy = $state(false)
  let status = $state('')
  // A baseline snapshot (workspace path -> size) taken at lesson start, so the teacher can see which
  // files a student added or changed. Size-based, never time-based (a student can change the clock).
  let baseline = $state<Record<string, number> | null>(null)

  function baselineKey(id: string): string {
    return `cowatcher.filebaseline.${id}`
  }
  function loadBaseline() {
    baseline = null
    if (!deviceId) return
    try {
      const raw = localStorage.getItem(baselineKey(deviceId))
      if (raw) baseline = JSON.parse(raw)
    } catch {
      /* ignore */
    }
  }
  // Re-load the baseline whenever the chosen PC changes.
  $effect(() => {
    void deviceId
    loadBaseline()
  })

  function entryPath(entry: FileEntry): string {
    return [...segments, entry.name].join('/')
  }
  // '' | 'new' | 'changed' relative to the baseline (empty when no baseline is set).
  function diff(entry: FileEntry): string {
    if (!baseline || entry.is_dir) return ''
    const path = entryPath(entry)
    if (!(path in baseline)) return 'new'
    return baseline[path] !== entry.bytes ? 'changed' : ''
  }

  // Pick the first live PC by default, and load its root when the dialog opens or the PC changes.
  $effect(() => {
    if (!deviceId && live.length > 0) deviceId = live[0].device_id
  })

  async function refresh() {
    if (!deviceId) {
      entries = []
      return
    }
    loading = true
    try {
      entries = await invoke<FileEntry[]>('list_files', { deviceId, dir })
    } catch (e) {
      entries = []
      onerror(String(e))
    } finally {
      loading = false
    }
  }

  // Reload whenever the chosen PC or directory changes.
  $effect(() => {
    // reference the deps so the effect re-runs on change
    void deviceId
    void dir
    refresh()
  })

  function open(entry: FileEntry) {
    if (entry.is_dir) segments = [...segments, entry.name]
  }
  function up() {
    segments = segments.slice(0, -1)
  }

  async function download(entry: FileEntry) {
    if (busy) return
    busy = true
    status = ''
    try {
      const path = [...segments, entry.name].join('/')
      const saved = await invoke<string>('download_file', { deviceId, path })
      status = t('filesSaved', saved)
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function pickAndSend(event: Event) {
    const input = event.target as HTMLInputElement
    const file = input.files?.[0]
    if (!file || !deviceId || busy) return
    busy = true
    status = ''
    try {
      const bytes = new Uint8Array(await file.arrayBuffer())
      await invoke('send_file', { deviceId, dir, name: file.name, data: Array.from(bytes) })
      status = t('filesSent', file.name)
      input.value = ''
      await refresh()
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function deleteEntry(entry: FileEntry) {
    if (busy) return
    if (!confirm(t('filesDeleteConfirm', entry.name))) return
    busy = true
    status = ''
    try {
      await invoke('delete_file', { deviceId, path: entryPath(entry) })
      status = t('filesDeleted', entry.name)
      await refresh()
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function wipeAll() {
    if (busy || !deviceId) return
    if (!confirm(t('filesWipeConfirm'))) return
    busy = true
    status = ''
    try {
      const removed = await invoke<number>('clear_workspace', { deviceId })
      status = t('filesWiped', removed)
      // The workspace is now empty, so the old baseline is meaningless.
      try {
        localStorage.removeItem(baselineKey(deviceId))
      } catch {
        /* ignore */
      }
      baseline = null
      await refresh()
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function markBaseline() {
    if (busy || !deviceId) return
    busy = true
    status = ''
    try {
      const files = await invoke<FileEntry[]>('workspace_manifest', { deviceId })
      const map: Record<string, number> = {}
      for (const f of files) map[f.name] = f.bytes
      try {
        localStorage.setItem(baselineKey(deviceId), JSON.stringify(map))
      } catch {
        /* ignore */
      }
      baseline = map
      status = t('filesBaselineSet', files.length)
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function collectAll() {
    if (busy || !deviceId) return
    busy = true
    status = ''
    try {
      const files = await invoke<FileEntry[]>('workspace_manifest', { deviceId })
      let saved = ''
      for (const f of files) {
        saved = await invoke<string>('download_file', { deviceId, path: f.name })
      }
      status = files.length > 0 ? t('filesCollected', files.length, saved) : t('filesEmpty')
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  function fmtSize(n: number): string {
    if (n < 1024) return `${n} B`
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
    return `${(n / (1024 * 1024)).toFixed(1)} MB`
  }

  function onkey(event: KeyboardEvent) {
    if (event.key === 'Escape') onclose()
  }
</script>

<svelte:window on:keydown={onkey} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('filesTitle')}>
    <h2>{t('filesTitle')}</h2>
    <p class="lead">{t('filesLead')}</p>

    <div class="row">
      <label class="pc">
        <span>{t('filesPickPc')}</span>
        <select bind:value={deviceId}>
          {#if live.length === 0}
            <option value="">{t('filesNoPc')}</option>
          {/if}
          {#each live as device (device.device_id)}
            <option value={device.device_id}>{device.name || device.device_id}</option>
          {/each}
        </select>
      </label>
      <label class="send">
        <input type="file" onchange={pickAndSend} disabled={!deviceId || busy} />
        <span>{t('filesSend')}</span>
      </label>
    </div>

    <div class="crumbs">
      <button class="up" onclick={up} disabled={segments.length === 0}>↑ {t('filesUp')}</button>
      <code class="path">{t('filesRoot')}{dir ? `/${dir}` : ''}</code>
      <span class="spacer"></span>
      <button class="act" onclick={markBaseline} disabled={!deviceId || busy}>{t('filesBaseline')}</button>
      <button class="act" onclick={collectAll} disabled={!deviceId || busy}>{t('filesCollect')}</button>
      <button class="act danger" onclick={wipeAll} disabled={!deviceId || busy}>{t('filesWipe')}</button>
    </div>

    <div class="list">
      {#if loading}
        <p class="hint">{t('loading')}</p>
      {:else if entries.length === 0}
        <p class="hint">{t('filesEmpty')}</p>
      {:else}
        {#each entries as entry (entry.name)}
          {@const mark = diff(entry)}
          <div class="item">
            {#if entry.is_dir}
              <button class="entry dir" onclick={() => open(entry)}>
                <span class="ic">📁</span><span class="nm">{entry.name}</span>
              </button>
            {:else}
              <span class="entry file"><span class="ic">📄</span><span class="nm">{entry.name}</span></span>
              {#if mark === 'new'}<span class="badge new">{t('filesNew')}</span>{/if}
              {#if mark === 'changed'}<span class="badge changed">{t('filesChanged')}</span>{/if}
              <span class="sz">{fmtSize(entry.bytes)}</span>
              <button class="dl" onclick={() => download(entry)} disabled={busy}>{t('filesDownload')}</button>
              <button class="del" onclick={() => deleteEntry(entry)} disabled={busy} aria-label={t('filesDelete')} title={t('filesDelete')}>✕</button>
            {/if}
          </div>
        {/each}
      {/if}
    </div>

    <footer>
      <span class="status">{status}</span>
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
    width: min(640px, 96vw);
    max-height: 92vh;
    overflow: hidden;
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
  .row {
    display: flex;
    gap: 12px;
    align-items: flex-end;
    flex-wrap: wrap;
    margin-bottom: 10px;
  }
  .pc {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--muted);
    flex: 1;
    min-width: 160px;
  }
  .pc select {
    padding: 7px 9px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .send {
    position: relative;
    display: grid;
    place-items: center;
    padding: 8px 14px;
    background: var(--accent);
    border-radius: 9px;
    color: #06101f;
    font-weight: 600;
    font-size: 13px;
    cursor: pointer;
  }
  .send input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }
  .send:has(input:disabled) {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .crumbs {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 8px;
  }
  .up {
    padding: 4px 10px;
    font-size: 12px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    cursor: pointer;
  }
  .path {
    font-size: 12px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list {
    flex: 1;
    overflow: auto;
    min-height: 160px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 6px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 4px;
  }
  .entry {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
    padding: 4px 6px;
    background: transparent;
    border: 0;
    border-radius: 6px;
    color: var(--text);
    font: inherit;
    font-size: 13px;
    text-align: left;
  }
  .entry.dir {
    cursor: pointer;
  }
  .entry.dir:hover {
    background: var(--hover);
  }
  .nm {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sz {
    font-size: 11.5px;
    color: var(--muted);
    flex: none;
  }
  .dl {
    flex: none;
    padding: 3px 10px;
    font-size: 12px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 7px;
    color: var(--text);
    cursor: pointer;
  }
  .dl:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .del {
    flex: none;
    width: 26px;
    padding: 3px 0;
    font-size: 12px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 7px;
    color: var(--muted);
    cursor: pointer;
  }
  .del:hover:not(:disabled) {
    color: var(--danger);
    border-color: var(--danger);
  }
  .act {
    padding: 4px 10px;
    font-size: 12px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    cursor: pointer;
  }
  .act:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .act.danger:hover:not(:disabled) {
    color: var(--danger);
    border-color: var(--danger);
  }
  .badge {
    flex: none;
    padding: 1px 7px;
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.3px;
    border-radius: 999px;
  }
  .badge.new {
    color: #06101f;
    background: var(--live, #3ecf8e);
  }
  .badge.changed {
    color: #06101f;
    background: var(--accent, #6ea8fe);
  }
  .hint {
    color: var(--muted);
    font-size: 12.5px;
    padding: 10px;
    text-align: center;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 14px;
    margin-top: 14px;
    border-top: 1px solid var(--line);
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
