<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { t } from './i18n.svelte'
  import type { Device } from './types'
  import ActionMenu from './ActionMenu.svelte'
  import ActionResult from './ActionResult.svelte'
  import RecordButton from './RecordButton.svelte'
  import AppsDialog from './AppsDialog.svelte'

  let {
    device,
    onclose,
    onmonitor,
    listening,
    onlisten,
    controlling,
    oncontrol,
    onerror,
  }: {
    device: Device
    onclose: () => void
    onmonitor: (index: number) => void
    listening: boolean
    onlisten: (on: boolean) => void
    controlling: boolean
    oncontrol: (on: boolean) => void
    onerror: (message: string) => void
  } = $props()

  let showApps = $state(false)
  let examOn = $state(false)

  async function toggleExam() {
    try {
      const [locked, problem] = await invoke<[boolean, string]>('set_exam', {
        deviceId: device.device_id,
        on: !examOn,
        message: t('examMessage'),
      })
      examOn = locked
      if (!locked && problem) onerror(problem)
    } catch (e) {
      onerror(String(e))
    }
  }

  // Full-resolution native viewer (ADR D12): the small JPEG below is the grid preview; this opens a
  // real window that decodes the live H.264 stream and can drive the PC. Resolution and rate are the
  // teacher's to pick — a 4K classroom projector may want 3840×2160 @ 15, a laptop 1280×720 @ 30.
  let res = $state('1920x1080')
  let fps = $state(30)
  const KBPS: Record<string, number> = {
    '1280x720': 2500,
    '1920x1080': 5000,
    '2560x1440': 9000,
    '3840x2160': 16000,
  }

  async function openViewer(control: boolean) {
    const [width, height] = res.split('x').map(Number)
    try {
      await invoke('open_viewer', {
        deviceId: device.device_id,
        width,
        height,
        fps: Math.max(1, Math.min(60, fps)),
        kbps: KBPS[res] ?? 5000,
        control,
      })
    } catch (e) {
      onerror(String(e))
    }
  }

  // Driving the PC from this preview: the picture below is a live JPEG, and these forward the
  // teacher's mouse and keyboard to the student over the same input path the native viewer uses.
  let imgEl: HTMLImageElement | undefined = $state()
  let lastMove = 0

  function sendInput(events: unknown[]) {
    invoke('send_input', { events }).catch((e) => onerror(String(e)))
  }

  // Cursor position as a 0–1 fraction of the *picture* (not the window), so a different resolution
  // on the student's side changes nothing.
  function fraction(event: PointerEvent) {
    if (!imgEl) return null
    const r = imgEl.getBoundingClientRect()
    if (r.width <= 0 || r.height <= 0) return null
    return {
      x: Math.min(1, Math.max(0, (event.clientX - r.left) / r.width)),
      y: Math.min(1, Math.max(0, (event.clientY - r.top) / r.height)),
    }
  }

  const BUTTONS = ['left', 'middle', 'right'] // PointerEvent.button 0 / 1 / 2

  function onPointerMove(event: PointerEvent) {
    if (!controlling) return
    const now = performance.now()
    if (now - lastMove < 15) return // ~60 Hz is plenty; do not flood the channel
    lastMove = now
    const f = fraction(event)
    if (f) sendInput([{ kind: 'move', x: f.x, y: f.y }])
  }

  function onPointerDown(event: PointerEvent) {
    if (!controlling) return
    const button = BUTTONS[event.button]
    if (!button) return
    const f = fraction(event)
    const events: unknown[] = []
    if (f) events.push({ kind: 'move', x: f.x, y: f.y })
    events.push({ kind: 'button', button, down: true })
    sendInput(events)
    // Capture the pointer so a release outside the picture still reaches us (no stuck button).
    try {
      ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
    } catch {
      // capture unsupported; releasing over the picture still works
    }
    event.preventDefault()
  }

  function onPointerUp(event: PointerEvent) {
    if (!controlling) return
    const button = BUTTONS[event.button]
    if (!button) return
    sendInput([{ kind: 'button', button, down: false }])
    try {
      ;(event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId)
    } catch {
      // nothing to release
    }
    event.preventDefault()
  }

  function onWheel(event: WheelEvent) {
    if (!controlling) return
    const delta = Math.max(-30, Math.min(30, Math.round(-event.deltaY / 40)))
    if (delta !== 0) sendInput([{ kind: 'scroll', delta }])
    event.preventDefault()
  }

  // Never forward keys the teacher is typing into a field of the console itself.
  function typingInConsole(event: KeyboardEvent) {
    const tag = (event.target as HTMLElement | null)?.tagName
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT'
  }

  function onKeyDown(event: KeyboardEvent) {
    if (controlling) {
      // Ctrl+Alt+Esc releases control (matches platform::input::KeyGate); never forwarded.
      if (event.key === 'Escape' && event.ctrlKey && event.altKey) {
        event.preventDefault()
        oncontrol(false)
        return
      }
      if (typingInConsole(event)) return
      // keyCode is the legacy Windows virtual-key code, which is exactly what the Agent wants.
      if (event.keyCode) sendInput([{ kind: 'key', virtualKey: event.keyCode, down: true }])
      else if (event.key.length === 1) sendInput([{ kind: 'text', text: event.key }])
      event.preventDefault()
      return
    }
    if (event.key === 'Escape') onclose()
  }

  function onKeyUp(event: KeyboardEvent) {
    if (!controlling || typingInConsole(event)) return
    if (event.keyCode) {
      sendInput([{ kind: 'key', virtualKey: event.keyCode, down: false }])
      event.preventDefault()
    }
  }
</script>

<svelte:window on:keydown={onKeyDown} on:keyup={onKeyUp} />

<!-- One screen, as large as the window allows: what the teacher opens to actually look at a PC. -->
<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="frame" role="dialog" aria-modal="true" aria-label={t('screenOf', device.device_id)}>
    <header>
      {#if device.name}
        <span class="name">{device.name}</span>
      {/if}
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

      <button
        class="listen"
        class:on={listening}
        onclick={() => onlisten(!listening)}
        title={t('listenHint')}
      >
        {listening ? t('listenStop') : t('listenStart')}
      </button>
      <button
        class="listen"
        class:on={controlling}
        onclick={() => oncontrol(!controlling)}
        title={t('controlHint')}
      >
        {controlling ? t('controlStop') : t('controlStart')}
      </button>
      <span class="liveview" role="group" aria-label={t('liveView')}>
        <select bind:value={res} aria-label={t('resolution')} title={t('resolution')}>
          <option value="1280x720">720p</option>
          <option value="1920x1080">1080p</option>
          <option value="2560x1440">1440p</option>
          <option value="3840x2160">4K</option>
        </select>
        <input
          type="number"
          min="1"
          max="60"
          bind:value={fps}
          aria-label={t('fpsLabel')}
          title={t('fpsLabel')}
        />
        <button onclick={() => openViewer(false)} title={t('liveViewHint')}>{t('liveView')}</button>
        <button onclick={() => openViewer(true)} title={t('liveControlHint')}>{t('liveControl')}</button>
      </span>
      <button onclick={() => (showApps = true)}>{t('appsButton')}</button>
      <button class="listen" class:on={examOn} onclick={toggleExam} title={t('examHint')}>
        {examOn ? t('examStop') : t('examStart')}
      </button>
      <RecordButton deviceId={device.device_id} {onerror} />
      <ActionResult report={device.last_action} />
      <ActionMenu deviceId={device.device_id} liveCount={device.status === 'live' ? 1 : 0} {onerror} />
      <button onclick={onclose}>{t('close')}</button>
    </header>
    <div class="screen">
      {#if device.screen}
        <img
          bind:this={imgEl}
          class:driving={controlling}
          src={device.screen}
          alt={t('screenOf', device.device_id)}
          draggable="false"
          onpointermove={onPointerMove}
          onpointerdown={onPointerDown}
          onpointerup={onPointerUp}
          onwheel={onWheel}
        />
      {:else}
        <p class="hint">{t('waitingFirst')}</p>
      {/if}
    </div>
  </div>
</div>

{#if showApps}
  <AppsDialog deviceId={device.device_id} onclose={() => (showApps = false)} {onerror} />
{/if}

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

  .name {
    font-size: 14px;
    font-weight: 600;
  }

  .id {
    font-family: ui-monospace, "Cascadia Mono", Consolas, monospace;
    font-size: 14px;
    color: var(--muted);
  }

  .liveview {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .liveview select,
  .liveview input {
    padding: 4px 6px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--text);
    font-size: 12px;
  }

  .liveview input {
    width: 3.5em;
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

  .listen {
    margin-left: auto;
  }

  .listen.on {
    background: var(--live);
    border-color: var(--live);
    color: #04150d;
    font-weight: 600;
  }

  .monitors ~ .listen {
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
    user-select: none;
    -webkit-user-drag: none;
  }

  /* While driving, the picture takes the pointer and a crosshair shows the teacher is in control. */
  img.driving {
    cursor: crosshair;
    outline: 2px solid var(--live);
    outline-offset: -2px;
  }

  .hint {
    color: var(--muted);
    font-size: 13px;
  }
</style>
