<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { listen as tauriListen, type UnlistenFn } from '@tauri-apps/api/event'
  import { onMount, onDestroy, tick } from 'svelte'
  import { t } from './i18n.svelte'

  // Surey: a floating, draggable/dockable AI assistant panel (Notion/VS Code side-panel style).
  // It chats with a user-configured provider and can act on the class through Rust-side tools.
  let { onclose }: { onclose: () => void } = $props()

  type Msg = { role: 'user' | 'assistant'; content: string }
  type Choice = { id: string; prompt: string; options: string[]; allow_custom: boolean }
  type Config = { kind: string; base_url: string; model: string; has_key: boolean }

  // ---- placement (persisted per browser) -------------------------------------------------------
  type Dock = 'float' | 'left' | 'right'
  let dock = $state<Dock>('right')
  let pos = $state({ x: window.innerWidth - 420, y: 80 })
  let size = $state({ w: 380, h: 560 })

  function loadPlacement() {
    try {
      const raw = localStorage.getItem('surey.placement')
      if (raw) {
        const p = JSON.parse(raw)
        dock = p.dock ?? dock
        pos = p.pos ?? pos
        size = p.size ?? size
      }
    } catch {
      /* first run / private window */
    }
  }
  function savePlacement() {
    try {
      localStorage.setItem('surey.placement', JSON.stringify({ dock, pos, size }))
    } catch {
      /* ignore */
    }
  }

  // ---- sessions (persisted) --------------------------------------------------------------------
  type Session = { id: string; title: string; messages: Msg[] }
  let sessions = $state<Session[]>([])
  let currentId = $state('')
  let current = $derived(sessions.find((s) => s.id === currentId) ?? null)

  function loadSessions() {
    try {
      const raw = localStorage.getItem('surey.sessions')
      if (raw) sessions = JSON.parse(raw)
    } catch {
      /* ignore */
    }
    if (sessions.length === 0) newSession()
    else currentId = sessions[0].id
  }
  function saveSessions() {
    try {
      localStorage.setItem('surey.sessions', JSON.stringify(sessions.slice(0, 30)))
    } catch {
      /* ignore */
    }
  }
  function newSession() {
    const s: Session = { id: crypto.randomUUID(), title: t('sureyNewChat'), messages: [] }
    sessions = [s, ...sessions]
    currentId = s.id
  }
  function deleteSession(id: string) {
    sessions = sessions.filter((s) => s.id !== id)
    if (sessions.length === 0) newSession()
    else if (!sessions.some((s) => s.id === currentId)) currentId = sessions[0].id
    saveSessions()
  }

  // ---- chat ------------------------------------------------------------------------------------
  let input = $state('')
  let busy = $state(false)
  let activity = $state('') // transient "Surey is doing X"
  let pendingChoice = $state<Choice | null>(null)
  let highlight = $state(0)
  let customChoice = $state('')
  let showSettings = $state(false)
  let messagesEl = $state<HTMLDivElement | null>(null)

  async function scrollDown() {
    await tick()
    messagesEl?.scrollTo({ top: messagesEl.scrollHeight, behavior: 'smooth' })
  }

  async function send() {
    const text = input.trim()
    if (!text || busy || !current) return
    input = ''
    current.messages.push({ role: 'user', content: text })
    sessions = sessions
    if (current.title === t('sureyNewChat')) current.title = text.slice(0, 40)
    saveSessions()
    scrollDown()

    busy = true
    activity = t('sureyThinking')
    try {
      const reply = await invoke<string>('ai_send', {
        messages: current.messages.map((m) => ({ role: m.role, content: m.content })),
      })
      current.messages.push({ role: 'assistant', content: reply || '…' })
      sessions = sessions
      saveSessions()
      scrollDown()
    } catch (e) {
      current.messages.push({ role: 'assistant', content: `⚠ ${String(e)}` })
      sessions = sessions
    } finally {
      busy = false
      activity = ''
      pendingChoice = null
    }
  }

  // ---- selection tool --------------------------------------------------------------------------
  async function choose(value: string) {
    if (!pendingChoice) return
    const id = pendingChoice.id
    pendingChoice = null
    customChoice = ''
    activity = t('sureyThinking')
    await invoke('ai_choice_reply', { id, value })
  }

  function onChoiceKey(event: KeyboardEvent) {
    if (!pendingChoice) return
    const n = pendingChoice.options.length
    if (event.key >= '1' && event.key <= '9') {
      const i = Number(event.key) - 1
      if (i < n) {
        event.preventDefault()
        choose(pendingChoice.options[i])
      }
    } else if (event.key === 'ArrowDown') {
      event.preventDefault()
      highlight = Math.min(highlight + 1, n - 1)
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      highlight = Math.max(highlight - 1, 0)
    } else if (event.key === 'Enter' && !pendingChoice.allow_custom) {
      event.preventDefault()
      choose(pendingChoice.options[highlight])
    }
  }

  // ---- voice: record with MediaRecorder, then transcribe through the provider (Whisper-shape). ---
  // Reliable everywhere getUserMedia works, and the recognised text lands in the box so you can see
  // and edit what was heard before sending. (WebView2 has no built-in speech service, so the browser
  // SpeechRecognition API is not used here — it silently ends with no result.)
  let listening = $state(false)
  let voiceSupported = $state(false)
  let recorder: MediaRecorder | null = null
  let micStream: MediaStream | null = null
  let chunks: Blob[] = []

  function setupVoice() {
    voiceSupported = !!navigator.mediaDevices?.getUserMedia && typeof MediaRecorder !== 'undefined'
  }

  async function toggleVoice() {
    if (listening) {
      // Stop → the recorder's onstop transcribes.
      recorder?.stop()
      listening = false
      return
    }
    try {
      micStream = await navigator.mediaDevices.getUserMedia({ audio: true })
      recorder = new MediaRecorder(micStream)
      chunks = []
      recorder.ondataavailable = (e) => {
        if (e.data.size > 0) chunks.push(e.data)
      }
      recorder.onstop = async () => {
        micStream?.getTracks().forEach((tr) => tr.stop())
        micStream = null
        const blob = new Blob(chunks, { type: recorder?.mimeType || 'audio/webm' })
        if (blob.size === 0) return
        const buf = new Uint8Array(await blob.arrayBuffer())
        activity = t('sureyTranscribing')
        try {
          const text = await invoke<string>('ai_transcribe', {
            audio: Array.from(buf),
            filename: 'voice.webm',
          })
          input = input ? `${input} ${text}` : text
          activity = ''
        } catch (e) {
          activity = `⚠ ${String(e)}`
        }
      }
      recorder.start()
      listening = true
    } catch (e) {
      activity = `⚠ ${String(e)}`
      listening = false
    }
  }

  // ---- settings --------------------------------------------------------------------------------
  let cfg = $state<Config>({ kind: 'openai', base_url: '', model: '', has_key: false })
  let keyInput = $state('')
  let models = $state<string[]>([])
  let settingsMsg = $state('')

  // ---- Co-Watcher subscription (placeholder; endpoint wired later) ------------------------------
  type Sub = { dashboard_url: string; has_license: boolean; plan: string }
  let sub = $state<Sub>({ dashboard_url: '', has_license: false, plan: 'free' })
  let licenseInput = $state('')
  let subMsg = $state('')

  async function loadConfig() {
    try {
      cfg = await invoke<Config>('ai_config')
    } catch (e) {
      settingsMsg = String(e)
    }
    try {
      sub = await invoke<Sub>('subscription_config')
    } catch {
      /* placeholder unavailable */
    }
  }

  async function saveSubscription() {
    subMsg = ''
    try {
      await invoke('subscription_set', {
        dashboardUrl: sub.dashboard_url,
        license: licenseInput ? licenseInput : null,
      })
      licenseInput = ''
      sub = await invoke<Sub>('subscription_config')
      subMsg = t('sureySaved')
    } catch (e) {
      subMsg = String(e)
    }
  }

  async function openDashboard() {
    subMsg = ''
    try {
      await invoke('open_dashboard')
    } catch (e) {
      subMsg = String(e)
    }
  }

  function onKindChange() {
    const presets: Record<string, string> = {
      openai: 'https://api.openai.com/v1',
      anthropic: 'https://api.anthropic.com/v1',
      local: 'http://localhost:11434/v1',
      custom: cfg.base_url || '',
    }
    cfg.base_url = presets[cfg.kind] ?? cfg.base_url
  }

  async function saveConfig() {
    settingsMsg = ''
    try {
      await invoke('ai_set_config', {
        kind: cfg.kind,
        baseUrl: cfg.base_url,
        model: cfg.model,
        key: keyInput ? keyInput : null,
      })
      keyInput = ''
      await loadConfig()
      settingsMsg = t('sureySaved')
    } catch (e) {
      settingsMsg = String(e)
    }
  }

  async function refreshModels() {
    settingsMsg = t('sureyLoadingModels')
    try {
      models = await invoke<string[]>('ai_list_models')
      settingsMsg = t('sureyModelsFound', models.length)
    } catch (e) {
      settingsMsg = String(e)
    }
  }

  // ---- drag / dock -----------------------------------------------------------------------------
  let dragging = false
  let dragOff = { x: 0, y: 0 }

  function startDrag(event: PointerEvent) {
    if (dock !== 'float') return
    dragging = true
    dragOff = { x: event.clientX - pos.x, y: event.clientY - pos.y }
    ;(event.target as HTMLElement).setPointerCapture(event.pointerId)
  }
  function onDrag(event: PointerEvent) {
    if (!dragging) return
    pos = {
      x: Math.max(0, Math.min(window.innerWidth - 120, event.clientX - dragOff.x)),
      y: Math.max(0, Math.min(window.innerHeight - 60, event.clientY - dragOff.y)),
    }
  }
  function endDrag() {
    if (dragging) {
      dragging = false
      savePlacement()
    }
  }
  function setDock(d: Dock) {
    dock = d
    savePlacement()
  }

  const style = $derived(
    dock === 'float'
      ? `left:${pos.x}px; top:${pos.y}px; width:${size.w}px; height:${size.h}px;`
      : dock === 'left'
        ? `left:0; top:0; bottom:0; width:${size.w}px; height:auto;`
        : `right:0; top:0; bottom:0; width:${size.w}px; height:auto;`,
  )

  // ---- lifecycle -------------------------------------------------------------------------------
  let unlisteners: UnlistenFn[] = []
  onMount(async () => {
    loadPlacement()
    loadSessions()
    setupVoice()
    await loadConfig()
    unlisteners.push(
      await tauriListen<{ name: string }>('surey://tool', (e) => {
        activity = t('sureyUsing', e.payload.name)
      }),
    )
    unlisteners.push(
      await tauriListen<Choice>('surey://choice', (e) => {
        pendingChoice = e.payload
        highlight = 0
        activity = ''
      }),
    )
    unlisteners.push(await tauriListen('surey://done', () => (activity = '')))
  })
  onDestroy(() => {
    unlisteners.forEach((u) => u())
    recorder?.stop?.()
    micStream?.getTracks().forEach((tr) => tr.stop())
  })

  function onGlobalKey(event: KeyboardEvent) {
    if (pendingChoice) onChoiceKey(event)
    else if (event.key === 'Escape' && !showSettings) onclose()
  }
</script>

<svelte:window
  on:keydown={onGlobalKey}
  on:pointermove={onDrag}
  on:pointerup={endDrag}
  on:resize={() => dock !== 'float' || savePlacement()}
/>

<div class="surey" class:float={dock === 'float'} {style} role="dialog" aria-label="Surey">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header class="bar" onpointerdown={startDrag}>
    <span class="who">
      <span class="dot"></span> Surey
    </span>
    <span class="tools">
      <button title={t('sureyNewChat')} onclick={newSession} aria-label={t('sureyNewChat')}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
      </button>
      <button
        class:active={showSettings}
        title={t('sureySettings')}
        onclick={() => (showSettings = !showSettings)}
        aria-label={t('sureySettings')}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="12" cy="12" r="3" />
          <path
            d="M19.4 13.5a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1 1.56V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1.11-1.56 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.56-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.56-1.11 1.7 1.7 0 0 0-.34-1.87l-.06-.06A2 2 0 1 1 7.09 4.4l.06.06a1.7 1.7 0 0 0 1.87.34H9a1.7 1.7 0 0 0 1-1.56V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.56 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87V9a1.7 1.7 0 0 0 1.56 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1.5Z"
          />
        </svg>
      </button>
      <button class:active={dock === 'left'} title="Dock left" onclick={() => setDock('left')} aria-label="Dock left">
        <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="16" height="16" rx="2" /><path d="M9 4v16" /></svg>
      </button>
      <button class:active={dock === 'float'} title="Float" onclick={() => setDock('float')} aria-label="Float">
        <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="6" y="6" width="14" height="14" rx="2" /><path d="M4 14V6a2 2 0 0 1 2-2h8" /></svg>
      </button>
      <button class:active={dock === 'right'} title="Dock right" onclick={() => setDock('right')} aria-label="Dock right">
        <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="5" y="4" width="16" height="16" rx="2" /><path d="M15 4v16" /></svg>
      </button>
      <button title={t('close')} onclick={onclose} aria-label={t('close')}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
    </span>
  </header>

  {#if showSettings}
    <div class="settings">
      <h3>{t('sureyProvider')}</h3>
      <label>
        {t('sureyProviderKind')}
        <select bind:value={cfg.kind} onchange={onKindChange}>
          <option value="openai">OpenAI</option>
          <option value="anthropic">Anthropic</option>
          <option value="custom">{t('sureyCustom')}</option>
          <option value="local">{t('sureyLocal')}</option>
        </select>
      </label>
      <label>
        {t('sureyBaseUrl')}
        <input bind:value={cfg.base_url} placeholder="https://api.openai.com/v1" />
      </label>
      <label>
        {t('sureyModel')}
        <span class="modelrow">
          <input bind:value={cfg.model} placeholder="gpt-4o-mini" list="surey-models" />
          <button onclick={refreshModels}>{t('sureyListModels')}</button>
        </span>
        <datalist id="surey-models">
          {#each models as m (m)}<option value={m}></option>{/each}
        </datalist>
      </label>
      <label>
        {t('sureyApiKey')}
        <input
          type="password"
          bind:value={keyInput}
          placeholder={cfg.has_key ? t('sureyKeySet') : 'sk-…'}
        />
      </label>
      <p class="hint">{t('sureyKeyHint')}</p>
      <div class="srow">
        <button class="primary" onclick={saveConfig}>{t('sureySave')}</button>
        <span class="smsg">{settingsMsg}</span>
      </div>

      <div class="subcard">
        <div class="subhead">
          <h3>{t('subTitle')}</h3>
          <span class="plan {sub.plan === 'custom' ? 'plan-custom' : 'plan-free'}">
            {sub.plan === 'custom' ? t('subPlanCustom') : t('subPlanFree')}
          </span>
        </div>
        <p class="hint">{t('subLead')}</p>
        <label>
          {t('subDashboardUrl')}
          <input bind:value={sub.dashboard_url} placeholder="https://dashboard.co-watcher..." />
        </label>
        <label>
          {t('subLicense')}
          <input
            type="password"
            bind:value={licenseInput}
            placeholder={sub.has_license ? t('sureyKeySet') : 'XXXX-XXXX-XXXX'}
          />
        </label>
        <div class="srow">
          <button class="primary" onclick={saveSubscription}>{t('sureySave')}</button>
          <button onclick={openDashboard} disabled={!sub.dashboard_url}>{t('subOpen')}</button>
          <span class="smsg">{subMsg}</span>
        </div>
      </div>
    </div>
  {:else}
    <div class="sessions">
      <select bind:value={currentId}>
        {#each sessions as s (s.id)}<option value={s.id}>{s.title}</option>{/each}
      </select>
      {#if sessions.length > 1}
        <button class="iconbtn" title={t('sureyDeleteChat')} aria-label={t('sureyDeleteChat')} onclick={() => deleteSession(currentId)}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2M6 7l1 13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-13" /></svg>
        </button>
      {/if}
    </div>

    <div class="messages" bind:this={messagesEl}>
      {#if current && current.messages.length === 0}
        <div class="greet">
          <p class="hi">{t('sureyGreeting')}</p>
          <p class="sub">{t('sureyGreetingSub')}</p>
        </div>
      {/if}
      {#each current?.messages ?? [] as m, i (i)}
        <div class="msg {m.role}">{m.content}</div>
      {/each}

      {#if pendingChoice}
        <div class="choice">
          <p class="cq">{pendingChoice.prompt}</p>
          {#each pendingChoice.options as opt, i (i)}
            <button class="opt" class:hl={i === highlight} onclick={() => choose(opt)}>
              <kbd>{i + 1}</kbd>
              {opt}
            </button>
          {/each}
          {#if pendingChoice.allow_custom}
            <form
              class="customrow"
              onsubmit={(e) => {
                e.preventDefault()
                if (customChoice.trim()) choose(customChoice.trim())
              }}
            >
              <input bind:value={customChoice} placeholder={t('sureyCustomAnswer')} />
              <button class="primary">↵</button>
            </form>
          {/if}
          <p class="chint">{t('sureyChoiceHint')}</p>
        </div>
      {/if}

      {#if activity}
        <div class="activity"><span class="spinner"></span> {activity}</div>
      {/if}
    </div>

    {#if listening}
      <div class="reclabel">
        <span class="recdot"></span>
        {t('sureyListening')}
      </div>
    {/if}
    <form class="composer" onsubmit={(e) => (e.preventDefault(), send())}>
      <button
        type="button"
        class="mic"
        class:on={listening}
        disabled={!voiceSupported}
        title={voiceSupported ? t('sureyMic') : t('sureyMicUnavailable')}
        onclick={toggleVoice}
      >
        {#if listening}
          <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="2" fill="currentColor" stroke="none" /></svg>
        {:else}
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <rect x="9" y="3" width="6" height="11" rx="3" />
            <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
          </svg>
        {/if}
      </button>
      <textarea
        bind:value={input}
        rows="1"
        placeholder={t('sureyAsk')}
        onkeydown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault()
            send()
          }
        }}
      ></textarea>
      <button class="primary send" disabled={busy || !input.trim()} aria-label={t('sureyAsk')}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 12l16-8-5 16-3.5-6.5L4 12Z" fill="currentColor" stroke="none" /></svg>
      </button>
    </form>
  {/if}

  {#if dock === 'float'}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="resize"
      onpointerdown={(e) => {
        const sx = e.clientX,
          sy = e.clientY,
          sw = size.w,
          sh = size.h
        const move = (ev: PointerEvent) => {
          size = {
            w: Math.max(300, sw + (ev.clientX - sx)),
            h: Math.max(320, sh + (ev.clientY - sy)),
          }
        }
        const up = () => {
          window.removeEventListener('pointermove', move)
          window.removeEventListener('pointerup', up)
          savePlacement()
        }
        window.addEventListener('pointermove', move)
        window.addEventListener('pointerup', up)
      }}
    ></div>
  {/if}
</div>

<style>
  .surey {
    position: fixed;
    z-index: 60;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border: 1px solid var(--line);
    box-shadow: 0 18px 50px rgba(0, 0, 0, 0.45);
    overflow: hidden;
  }
  .surey.float {
    border-radius: 14px;
  }

  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px;
    background: linear-gradient(180deg, rgba(255, 255, 255, 0.06), transparent);
    border-bottom: 1px solid var(--line);
    cursor: grab;
    touch-action: none;
    user-select: none;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 700;
    letter-spacing: 0.2px;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--accent, #6ea8fe);
    box-shadow: 0 0 8px var(--accent, #6ea8fe);
  }
  /* Every icon in the panel is a stroke SVG, sized and aligned the same so nothing drifts. */
  .surey :global(svg) {
    stroke: currentColor;
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .tools button :global(svg) {
    display: block;
    width: 15px;
    height: 15px;
  }
  .tools {
    display: flex;
    gap: 2px;
  }
  .tools button {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 7px;
    color: var(--muted);
    cursor: pointer;
    font-size: 13px;
  }
  .tools button:hover {
    background: var(--bg);
    color: inherit;
  }
  .tools button.active {
    color: var(--accent, #6ea8fe);
    border-color: var(--line);
  }

  .sessions {
    display: flex;
    gap: 6px;
    padding: 8px 10px 0;
  }
  .sessions select {
    flex: 1;
    min-width: 0;
  }

  .messages {
    flex: 1;
    overflow: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .greet {
    margin: auto;
    text-align: center;
    color: var(--muted);
  }
  .greet .hi {
    font-size: 15px;
    font-weight: 600;
    margin: 0 0 4px;
  }
  .greet .sub {
    font-size: 12.5px;
    margin: 0;
  }

  .msg {
    max-width: 88%;
    padding: 8px 11px;
    border-radius: 12px;
    font-size: 13px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .msg.user {
    align-self: flex-end;
    background: var(--accent, #3b6fd4);
    color: #fff;
    border-bottom-right-radius: 4px;
  }
  .msg.assistant {
    align-self: flex-start;
    background: var(--bg);
    border: 1px solid var(--line);
    border-bottom-left-radius: 4px;
  }

  .choice {
    align-self: stretch;
    padding: 10px;
    background: var(--bg);
    border: 1px solid var(--accent, #6ea8fe);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .cq {
    margin: 0 0 2px;
    font-weight: 600;
    font-size: 13px;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 9px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 8px;
    cursor: pointer;
    text-align: left;
    font-size: 12.5px;
  }
  .opt.hl,
  .opt:hover {
    border-color: var(--accent, #6ea8fe);
  }
  .opt kbd {
    min-width: 18px;
    text-align: center;
    padding: 1px 4px;
    font-size: 11px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 5px;
  }
  .customrow {
    display: flex;
    gap: 6px;
  }
  .customrow input {
    flex: 1;
  }
  .chint,
  .hint {
    margin: 2px 0 0;
    color: var(--muted);
    font-size: 11px;
  }

  .activity {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--muted);
    font-size: 12px;
  }
  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--line);
    border-top-color: var(--accent, #6ea8fe);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .composer {
    display: flex;
    align-items: flex-end;
    gap: 6px;
    padding: 10px;
    border-top: 1px solid var(--line);
  }
  .composer textarea {
    flex: 1;
    resize: none;
    max-height: 120px;
    font: inherit;
    font-size: 13px;
    line-height: 1.4;
  }
  .mic,
  .send {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    flex: none;
    border-radius: 9px;
    cursor: pointer;
  }
  .mic :global(svg),
  .send :global(svg) {
    display: block;
    width: 18px;
    height: 18px;
  }
  .mic {
    color: var(--muted);
    background: var(--bg);
    border: 1px solid var(--line);
  }
  .mic:hover:not(:disabled) {
    color: inherit;
  }
  .mic.on {
    color: var(--danger, #e5484d);
    border-color: var(--danger, #e5484d);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .mic:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  @keyframes pulse {
    50% {
      box-shadow: 0 0 0 4px rgba(229, 72, 77, 0.25);
    }
  }

  .reclabel {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0 12px;
    padding: 6px 10px;
    font-size: 12px;
    color: var(--danger, #e5484d);
    background: rgba(229, 72, 77, 0.1);
    border-radius: 8px;
  }
  .recdot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--danger, #e5484d);
    animation: pulse 1.2s ease-in-out infinite;
  }

  .settings {
    padding: 12px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .settings h3 {
    margin: 0;
    font-size: 13px;
  }
  .settings label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--muted);
  }
  .settings input,
  .settings select {
    font: inherit;
    font-size: 13px;
    color: inherit;
  }
  .modelrow {
    display: flex;
    gap: 6px;
  }
  .modelrow input {
    flex: 1;
  }
  .srow {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .smsg {
    font-size: 12px;
    color: var(--muted);
  }

  .subcard {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 6px;
    padding: 12px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 12px;
  }
  .subhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .plan {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    padding: 3px 9px;
    border-radius: 999px;
  }
  .plan-free {
    color: var(--muted);
    border: 1px solid var(--line);
  }
  .plan-custom {
    color: #fff;
    background: var(--accent, #3b6fd4);
  }
  .subcard button:not(.primary) {
    padding: 6px 12px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 9px;
    color: inherit;
    cursor: pointer;
  }
  .subcard button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .sessions .iconbtn {
    display: grid;
    place-items: center;
    width: 32px;
    padding: 0;
    color: var(--muted);
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    cursor: pointer;
  }
  .sessions .iconbtn:hover {
    color: var(--danger, #e5484d);
    border-color: var(--danger, #e5484d);
  }
  .sessions .iconbtn :global(svg) {
    width: 15px;
    height: 15px;
  }

  .resize {
    position: absolute;
    right: 0;
    bottom: 0;
    width: 16px;
    height: 16px;
    cursor: nwse-resize;
    touch-action: none;
  }
  .resize::after {
    content: '';
    position: absolute;
    right: 3px;
    bottom: 3px;
    width: 7px;
    height: 7px;
    border-right: 2px solid var(--muted);
    border-bottom: 2px solid var(--muted);
  }

  .primary {
    background: var(--accent, #3b6fd4);
    color: #fff;
    border: none;
    border-radius: 9px;
    cursor: pointer;
  }
</style>
