<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { t } from './i18n.svelte'
  import type { AppEntry, RunningApp } from './types'

  /**
   * Start a program on a student PC, or close one that is running.
   *
   * The list of programs comes *from that PC* — this window can only pick from what the PC itself
   * published, never name a path. That is what keeps a launcher from being "run anything".
   */
  let {
    deviceId,
    onclose,
    onerror,
  }: { deviceId: string; onclose: () => void; onerror: (message: string) => void } = $props()

  let apps = $state<AppEntry[]>([])
  let running = $state<RunningApp[]>([])
  let filter = $state('')
  let loading = $state(true)
  let busy = $state(false)

  // Make this overlay a real movable, resizable window: dragging its title bar switches it from the
  // centered default to a fixed position, and CSS `resize` gives it a corner grip.
  let dialogEl = $state<HTMLDivElement>()
  let placed = $state<{ x: number; y: number } | null>(null)
  // Close on a click that both starts and ends on the backdrop, so releasing a resize/drag over the
  // backdrop never dismisses the window.
  let downOnBackdrop = false
  let dragging = false
  let start = { px: 0, py: 0, x: 0, y: 0 }

  function dragStart(e: PointerEvent) {
    if (!dialogEl) return
    const r = dialogEl.getBoundingClientRect()
    placed = { x: r.left, y: r.top }
    dragging = true
    start = { px: e.clientX, py: e.clientY, x: r.left, y: r.top }
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  }
  function dragMove(e: PointerEvent) {
    if (!dragging) return
    const nx = start.x + (e.clientX - start.px)
    const ny = start.y + (e.clientY - start.py)
    // Keep a sliver on screen so it can never be dragged fully out of reach.
    const maxX = window.innerWidth - 80
    const maxY = window.innerHeight - 40
    placed = { x: Math.max(0, Math.min(maxX, nx)), y: Math.max(0, Math.min(maxY, ny)) }
  }
  function dragEnd(e: PointerEvent) {
    dragging = false
    try {
      ;(e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId)
    } catch {
      // pointer already released; nothing to do
    }
  }

  const needle = $derived(filter.trim().toLowerCase())
  const matches = $derived(
    apps.filter((a) => a.name.toLowerCase().includes(needle)).slice(0, 80),
  )
  // The same search box also filters the running list, so a teacher can find a process by name.
  const runningMatches = $derived(running.filter((r) => r.name.toLowerCase().includes(needle)))

  async function load() {
    loading = true
    try {
      apps = await invoke<AppEntry[]>('list_apps', { deviceId })
      running = await invoke<RunningApp[]>('list_running', { deviceId })
    } catch (e) {
      onerror(String(e))
    } finally {
      loading = false
    }
  }
  load()

  // Keep the running list live: a program the student (or the teacher) started should appear on its
  // own within a couple of seconds, not only when Refresh is pressed. The installed-apps list is
  // re-fetched by Refresh, since it changes rarely.
  async function refreshRunning() {
    if (busy) return
    try {
      running = await invoke<RunningApp[]>('list_running', { deviceId })
    } catch {
      // A transient miss is not worth interrupting the teacher; the next tick retries.
    }
  }

  $effect(() => {
    const timer = setInterval(refreshRunning, 2000)
    return () => clearInterval(timer)
  })

  async function launch(app: AppEntry) {
    busy = true
    try {
      const started = await invoke<boolean>('launch_app', { deviceId, id: app.id })
      if (!started) onerror(t('appsFailed', app.name))
      // Give it a moment to appear before refreshing what is running.
      setTimeout(load, 800)
    } catch (e) {
      onerror(String(e))
    } finally {
      busy = false
    }
  }

  async function close(app: RunningApp) {
    busy = true
    try {
      await invoke<boolean>('close_app', { deviceId, pid: app.pid })
      running = running.filter((r) => r.pid !== app.pid)
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

<div
  class="backdrop"
  role="presentation"
  onpointerdown={(e) => (downOnBackdrop = e.target === e.currentTarget)}
  onclick={(e) => {
    if (downOnBackdrop && e.target === e.currentTarget) onclose()
  }}
>
  <div
    class="dialog"
    class:placed
    bind:this={dialogEl}
    role="dialog"
    aria-modal="true"
    aria-label={t('appsTitle')}
    style={placed ? `left:${placed.x}px; top:${placed.y}px;` : ''}
  >
    <h2
      class="draghandle"
      onpointerdown={dragStart}
      onpointermove={dragMove}
      onpointerup={dragEnd}
      onpointercancel={dragEnd}
    >
      {t('appsTitle', deviceId)}
    </h2>
    <p class="lead">{t('appsLead')}</p>

    <input bind:value={filter} placeholder={t('appsFilter')} aria-label={t('appsFilter')} />

    {#if loading}
      <p class="muted">{t('loading')}</p>
    {:else}
      <div class="columns">
        <section>
          <h3>{t('appsInstalled', apps.length)}</h3>
          <ul>
            {#each matches as app (app.id)}
              <li>
                <span class="name">{app.name}</span>
                <button onclick={() => launch(app)} disabled={busy}>{t('appsLaunch')}</button>
              </li>
            {/each}
            {#if matches.length === 0}
              <li class="muted">{t('appsNone')}</li>
            {/if}
          </ul>
        </section>

        <section>
          <h3>{t('appsRunning', runningMatches.length)}</h3>
          <ul>
            {#each runningMatches.slice(0, 80) as app (app.pid)}
              <li>
                <span class="name">{app.name}</span>
                <button class="danger" onclick={() => close(app)} disabled={busy}>
                  {t('appsClose')}
                </button>
              </li>
            {/each}
          </ul>
        </section>
      </div>
    {/if}

    <footer>
      <button onclick={load} disabled={busy}>{t('appsRefresh')}</button>
      <span class="spacer"></span>
      <button onclick={onclose}>{t('close')}</button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 30;
    display: grid;
    place-items: center;
    background: rgba(6, 9, 13, 0.82);
    padding: 18px;
  }

  .dialog {
    display: flex;
    flex-direction: column;
    width: min(680px, 96vw);
    height: min(560px, 88vh);
    min-width: 360px;
    min-height: 260px;
    max-width: 96vw;
    max-height: 92vh;
    padding: 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 14px;
    /* A real window: drag the title bar to move, drag the corner to resize. */
    resize: both;
    overflow: hidden;
  }

  /* Once dragged it detaches from the centered default and floats where the teacher put it. */
  .dialog.placed {
    position: fixed;
    margin: 0;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 17px;
  }

  .draghandle {
    cursor: move;
    user-select: none;
    touch-action: none;
  }

  h3 {
    margin: 0 0 6px;
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
  }

  .lead {
    margin: 0 0 12px;
    color: var(--muted);
    font-size: 13px;
  }

  input {
    padding: 8px 10px;
    margin-bottom: 12px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font-size: 13px;
  }

  .columns {
    display: grid;
    grid-template-columns: 1fr 1fr;
    grid-template-rows: minmax(0, 1fr);
    gap: 14px;
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
  }

  section {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 4px;
    flex: 1 1 auto;
    min-height: 0;
    align-content: start;
    overflow: auto;
  }

  li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
  }

  li.muted {
    background: transparent;
    border-color: transparent;
  }

  .name {
    flex: 1;
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .muted {
    color: var(--muted);
    font-size: 12.5px;
  }

  .danger:hover {
    border-color: var(--danger);
    color: var(--danger);
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 12px;
    margin-top: 12px;
    border-top: 1px solid var(--line);
  }

  .spacer {
    flex: 1;
  }

  @media (max-width: 560px) {
    .columns {
      grid-template-columns: 1fr;
    }
  }
</style>
