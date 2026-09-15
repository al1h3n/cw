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

  const matches = $derived(
    apps.filter((a) => a.name.toLowerCase().includes(filter.trim().toLowerCase())).slice(0, 80),
  )

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

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('appsTitle')}>
    <h2>{t('appsTitle', deviceId)}</h2>
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
          <h3>{t('appsRunning', running.length)}</h3>
          <ul>
            {#each running.slice(0, 80) as app (app.pid)}
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
    width: min(680px, 100%);
    max-height: 100%;
    padding: 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 14px;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 17px;
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
    gap: 14px;
    min-height: 0;
    overflow: hidden;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 4px;
    max-height: 46vh;
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
