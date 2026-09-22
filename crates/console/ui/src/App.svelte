<script lang="ts">
  import { invoke } from './lib/bridge'
  import { onDestroy, onMount, untrack } from 'svelte'
  import { listen as tauriListen, type UnlistenFn } from './lib/bridge'
  import { toasts } from './lib/toast-store.svelte'
  import { flip } from 'svelte/animate'
  import { slide, fade } from 'svelte/transition'
  import { cubicOut } from 'svelte/easing'
  import { groups, UNCAT } from './lib/groups.svelte'
  import DeviceTile from './lib/DeviceTile.svelte'
  import PairDialog from './lib/PairDialog.svelte'
  import BroadcastDialog from './lib/BroadcastDialog.svelte'
  import RecordingsDialog from './lib/RecordingsDialog.svelte'
  import WallpaperDialog from './lib/WallpaperDialog.svelte'
  import SureyPanel from './lib/SureyPanel.svelte'
  import SureyLauncher from './lib/SureyLauncher.svelte'
  import Focused from './lib/Focused.svelte'
  import ActionMenu from './lib/ActionMenu.svelte'
  import BlocklistDialog from './lib/BlocklistDialog.svelte'
  import Tutorial from './lib/Tutorial.svelte'
  import RoomCard from './lib/RoomCard.svelte'
  import LanguagePicker from './lib/LanguagePicker.svelte'
  import QualityPicker from './lib/QualityPicker.svelte'
  import Menu from './lib/Menu.svelte'
  import Icon from './lib/Icon.svelte'
  import SettingsDialog from './lib/SettingsDialog.svelte'
  import Toasts from './lib/Toasts.svelte'
  import { i18n, t } from './lib/i18n.svelte'
  import { applyTheme, DEFAULT_SETTINGS } from './lib/theme'
  import type { ConsoleInfo, Device, Settings, Classroom } from './lib/types'

  let info = $state<ConsoleInfo | null>(null)
  let devices = $state<Device[]>([])
  let watching = $state(false)
  let pairing = $state(false)
  let broadcasting = $state(false)
  let showRecordings = $state(false)
  let settingWallpaper = $state(false)
  let sureyOpen = $state(false)
  let editingBlocklist = $state(false)
  let showTutorial = $state(false)
  let showSettings = $state(false)
  // Right-click-anywhere menu of class-wide actions.
  let bgMenu = $state<{ x: number; y: number } | null>(null)
  // Whether a tile keeps showing its last screen after watching stops / it goes offline. Persisted.
  let keepPreviews = $state<boolean>(loadKeepPreviews())
  // Console-wide preferences (AI on/off, colour theme), loaded from the backend on mount.
  let settings = $state<Settings>(DEFAULT_SETTINGS)
  // Every classroom, for the header switcher. Switching opens a chosen one in a new window.
  let classrooms = $state<Classroom[]>([])
  let creatingClassroom = $state(false)
  let newClassroomName = $state('')
  const activeClassroomName = $derived(classrooms.find((c) => c.active)?.name ?? t('room'))

  async function saveSettings(next: Settings) {
    settings = next
    applyTheme(next)
    try {
      await invoke('settings_set', { settings: next })
    } catch (e) {
      toasts.push(String(e), 'error')
    }
  }

  async function switchClassroom(slug: string) {
    try {
      await invoke('switch_classroom', { slug })
    } catch (e) {
      toasts.push(String(e), 'error')
    }
  }

  async function submitNewClassroom() {
    const name = newClassroomName.trim()
    if (!name) return
    try {
      const room = await invoke<Classroom>('create_classroom', { name })
      classrooms = await invoke('classrooms')
      creatingClassroom = false
      // Open the new classroom in its own window (feature 1: switching is a new instance).
      await switchClassroom(room.slug)
    } catch (e) {
      toasts.push(String(e), 'error')
    }
  }

  function loadKeepPreviews(): boolean {
    try {
      return localStorage.getItem('cowatcher.keepPreviews') !== 'no'
    } catch {
      return true
    }
  }
  function setKeepPreviews(value: boolean) {
    keepPreviews = value
    try {
      localStorage.setItem('cowatcher.keepPreviews', value ? 'yes' : 'no')
    } catch {
      // storage unavailable; the choice just will not persist
    }
  }

  const liveDevices = $derived(devices.filter((d) => d.status === 'live').map((d) => d.device_id))

  /** Freeze or release every connected PC's own input (screen lock), without taking control. */
  async function screenLockAll(on: boolean) {
    for (const id of liveDevices) {
      try {
        await invoke('set_screen_lock', { deviceId: id, on })
      } catch (e) {
        error = String(e)
      }
    }
  }
  /** Exam-lock (or release) every connected PC at once. */
  async function examAll(on: boolean) {
    for (const id of liveDevices) {
      try {
        await invoke('set_exam', { deviceId: id, on, message: t('examMessage') })
      } catch (e) {
        error = String(e)
      }
    }
  }
  /** Apply the wallpaper-change policy (lock/unlock) to every connected PC. */
  async function wallpaperPolicy(action: 'lock-wallpaper' | 'unlock-wallpaper') {
    try {
      await invoke('perform', { deviceId: null, action, delaySeconds: 0 })
    } catch (e) {
      error = String(e)
    }
  }
  function openBgMenu(event: MouseEvent) {
    event.preventDefault()
    bgMenu = {
      x: Math.min(event.clientX, window.innerWidth - 210),
      y: Math.min(event.clientY, window.innerHeight - 260),
    }
  }
  let controllingId = $state<string | null>(null)
  let focused = $state<string | null>(null)
  let listeningTo = $state<string | null>(null)
  let error = $state<string | null>(null)
  let loaded = $state(false)
  let keyCopied = $state(false)
  // Grid tile size (px, the min column width). Bigger tiles are what actually make a higher grid
  // *resolution* visible — a small tile downscales any resolution to the same pixels. Persisted.
  let tileMin = $state<number>(loadTileMin())

  function loadTileMin(): number {
    try {
      const v = Number(localStorage.getItem('cowatcher.tile'))
      if (v >= 180 && v <= 640) return v
    } catch {
      // storage unavailable; fall through to the default
    }
    return 260
  }
  function setTileMin(v: number) {
    tileMin = v
    try {
      localStorage.setItem('cowatcher.tile', String(v))
    } catch {
      // storage unavailable; the size just will not persist across restarts
    }
  }
  // Broadcasts keep running after their picker dialog is closed — and several can run at once to
  // different PC groups — so a header banner lists each live one with its own Stop.
  type Presenting = { id: number; targets: number; label: string }
  let presenting = $state<Presenting[]>([])

  async function stopPresenting(id: number) {
    try {
      await invoke('stop_broadcast', { id })
    } catch (e) {
      error = String(e)
    }
    presenting = presenting.filter((p) => p.id !== id)
  }

  async function copyKey() {
    if (!info) return
    await navigator.clipboard.writeText(info.public_key)
    keyCopied = true
    setTimeout(() => (keyCopied = false), 1500)
  }
  let timer: number | undefined

  const live = $derived(devices.filter((d) => d.status === 'live').length)
  const focusedDevice = $derived(devices.find((d) => d.device_id === focused) ?? null)

  // --- Grouping: arrange tiles into named groups, drag to reorder / move, persisted locally. ---
  const byId = $derived(new Map(devices.map((d) => [d.device_id, d])))

  // Keep the arrangement in step with the real device list without re-triggering itself.
  $effect(() => {
    const ids = devices.map((d) => d.device_id)
    untrack(() => groups.sync(ids))
  })

  function tilesOf(groupId: string): Device[] {
    return groups
      .members(groupId)
      .map((id) => byId.get(id))
      .filter((d): d is Device => d !== undefined)
  }

  let draggingId = $state<string | null>(null)
  // Where the drop would land, shown as a highlight. Set on dragover, applied only on drop.
  let dropTarget = $state<{ groupId: string; index: number } | null>(null)
  let tileMenu = $state<{ id: string; x: number; y: number } | null>(null)
  let groupMenu = $state<{ id: string; x: number; y: number } | null>(null)
  let editingGroup = $state<string | null>(null)

  function dragStart(event: DragEvent, id: string) {
    draggingId = id
    const cell = (event.currentTarget as HTMLElement).closest('.cell') as HTMLElement | null
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = 'move'
      event.dataTransfer.setData('text/plain', id)
      if (cell) event.dataTransfer.setDragImage(cell, 24, 24)
    }
  }
  function dragEnd() {
    draggingId = null
    dropTarget = null
  }
  // IMPORTANT: dragover must NOT reorder the list. Reordering the DOM under the cursor while a flip
  // animation runs makes the dragged tile jump under the pointer, which fires a dragleave/dragover
  // storm that hard-freezes the whole webview (bug: "moving a tile out of a group freezes the app").
  // So dragover only records where the drop would land; the actual move happens once, on drop.
  function onCellDragOver(event: DragEvent, groupId: string, index: number) {
    if (!draggingId) return
    event.preventDefault()
    // Stop the event bubbling to the section's own dragover, which would immediately overwrite this
    // per-tile target with "drop at the end" — the bug where every drag landed at the end.
    event.stopPropagation()
    if (dropTarget?.groupId !== groupId || dropTarget?.index !== index) {
      dropTarget = { groupId, index }
    }
  }
  function onGroupDragOver(event: DragEvent, groupId: string) {
    if (!draggingId) return
    event.preventDefault()
    const end = groups.members(groupId).length
    if (dropTarget?.groupId !== groupId || dropTarget?.index !== end) {
      dropTarget = { groupId, index: end }
    }
  }
  function onDrop(event: DragEvent) {
    event.preventDefault()
    if (draggingId && dropTarget) {
      groups.move(draggingId, dropTarget.groupId, dropTarget.index)
      groups.save()
    }
    dragEnd()
  }

  function menuPos(event: MouseEvent) {
    return {
      x: Math.min(event.clientX, window.innerWidth - 190),
      y: Math.min(event.clientY, window.innerHeight - 240),
    }
  }
  function openTileMenu(event: MouseEvent, id: string) {
    event.preventDefault()
    event.stopPropagation()
    groupMenu = null
    tileMenu = { id, ...menuPos(event) }
  }
  function openGroupMenu(event: MouseEvent, id: string) {
    event.preventDefault()
    event.stopPropagation()
    tileMenu = null
    groupMenu = { id, ...menuPos(event) }
  }
  function closeMenus() {
    tileMenu = null
    groupMenu = null
    bgMenu = null
  }
  function moveToGroup(id: string, groupId: string) {
    groups.move(id, groupId, groups.members(groupId).length)
    groups.save()
    closeMenus()
  }
  function createGroup() {
    const id = groups.addGroup()
    editingGroup = id
    closeMenus()
  }
  function moveToNewGroup(id: string) {
    const gid = groups.addGroup()
    groups.move(id, gid, 0)
    groups.save()
    editingGroup = gid
    closeMenus()
  }
  function focusInput(node: HTMLInputElement) {
    node.focus()
    node.select()
  }

  async function refresh() {
    try {
      devices = await invoke<Device[]>('devices')
      loaded = true
    } catch (e) {
      error = String(e)
    }
    try {
      presenting = await invoke<Presenting[]>('broadcast_status')
    } catch {
      // Status is best-effort; leave the banner as it was on a transient miss.
    }
  }

  /** Listening is exclusive: starting one PC stops any other. */
  async function listen(deviceId: string | null) {
    try {
      await invoke('set_listening', { deviceId })
      listeningTo = deviceId
    } catch (e) {
      error = String(e)
    }
  }

  /** Opening a screen asks the backend to refresh it faster and larger. */
  async function open(deviceId: string | null) {
    focused = deviceId
    try {
      await invoke('set_focused', { deviceId })
    } catch (e) {
      error = String(e)
    }
  }

  async function wake(deviceId: string) {
    try {
      const count = await invoke<number>('wake', { deviceId })
      error = null
      if (count === 0) error = t('wakeNone', deviceId)
    } catch (e) {
      error = String(e)
    }
  }

  async function rename(deviceId: string, name: string) {
    try {
      await invoke('rename_device', { deviceId, name })
      await refresh()
    } catch (e) {
      error = String(e)
    }
  }

  async function chooseMonitor(deviceId: string, index: number) {
    try {
      await invoke('set_monitor', { deviceId, monitor: index })
      await refresh()
    } catch (e) {
      error = String(e)
    }
  }

  async function toggleWatching() {
    error = null
    try {
      if (watching) {
        await invoke('stop_watching')
        watching = false
        // Watching drives the connections, so listening cannot survive it being switched off.
        if (listeningTo) await listen(null)
      } else {
        await invoke('start_watching')
        watching = true
      }
    } catch (e) {
      error = String(e)
    }
  }

  /** Taking control is exclusive: the backend enforces it, this just tracks what to show. */
  async function control(deviceId: string | null) {
    try {
      await invoke('set_controlling', { deviceId })
      controllingId = deviceId
    } catch (e) {
      error = String(e)
    }
  }

  onMount(async () => {
    try {
      // Strings first, so nothing renders in the wrong language.
      await i18n.load()
      info = await invoke<ConsoleInfo>('console_info')
      // Preferences (AI on/off, theme) and the classroom list, so the theme applies before content.
      settings = await invoke<Settings>('settings_get')
      applyTheme(settings)
      classrooms = await invoke<Classroom[]>('classrooms')
    } catch (e) {
      error = String(e)
    }
    // The tour shows once, and only when there is nothing else demanding attention.
    try {
      showTutorial = localStorage.getItem('cowatcher.tutorial.seen') !== 'yes'
    } catch {
      showTutorial = false
    }
    await refresh()
    // One poll drives the whole grid; the agents only capture while watching is on. Twice a second
    // keeps the displayed frames close to live without re-rendering the grid too aggressively.
    timer = window.setInterval(refresh, 500)
    // A PC dropping the broadcast (closed, crashed or disconnected) toasts the teacher (bug #6).
    unlistenBroadcast = await tauriListen<string>('cowatcher://broadcast-ended', (e) => {
      const dev = devices.find((d) => d.device_id === e.payload)
      toasts.push(t('broadcastEndedOn', dev?.name || e.payload), 'error')
    })
    // The shared window closed and the teacher chose "stop": the backend already cleared the class, so
    // drop that broadcast's banner (by id) and let them know.
    unlistenSourceLost = await tauriListen<number>('cowatcher://broadcast-source-lost', (e) => {
      presenting = presenting.filter((p) => p.id !== e.payload)
      toasts.push(t('broadcastSourceLost'), 'info')
    })
  })

  let unlistenBroadcast: UnlistenFn | null = null
  let unlistenSourceLost: UnlistenFn | null = null
  onDestroy(() => {
    if (timer) window.clearInterval(timer)
    unlistenBroadcast?.()
    unlistenSourceLost?.()
  })
</script>

<!-- Nothing renders until the strings are in, so raw keys never flash on screen. -->
{#if !i18n.ready}
  <div class="boot"></div>
{:else}
<div class="shell">
  <header>
    <Menu label={activeClassroomName} align="left">
      {#snippet icon()}<Icon name="layers" />{/snippet}
      {#each classrooms as room (room.slug)}
        <button class="mi" class:on={room.active} onclick={() => !room.active && switchClassroom(room.slug)}>
          <Icon name="monitor" />{room.name}
        </button>
      {/each}
      <span class="sep"></span>
      <button class="mi" onclick={() => { newClassroomName = ''; creatingClassroom = true }}>
        <Icon name="plus" />{t('classroomNew')}
      </button>
    </Menu>
    <div class="title">
      <h1>{t('room')}</h1>
      <p class="sub">
        {#if loaded}
          {devices.length === 0 ? t('noDevices') : t('deviceCount', devices.length, live)}
        {:else}
          {t('loading')}
        {/if}
      </p>
    </div>

    <div class="actions">
      <button
        class="withicon"
        class:primary={!watching}
        onclick={toggleWatching}
        disabled={devices.length === 0}
        title={devices.length === 0 ? t('addFirst') : ''}
      >
        <Icon name={watching ? 'eye-off' : 'eye'} />
        {watching ? t('stopWatching') : t('startWatching')}
      </button>
      <button class="withicon" onclick={() => (pairing = true)}>
        <Icon name="plus" />
        {t('addPc')}
      </button>

      <Menu label={t('groupContent')} disabled={devices.length === 0}>
        {#snippet icon()}<Icon name="cast" />{/snippet}
        <button class="mi" onclick={() => (broadcasting = true)}>
          <Icon name="cast" />{t('broadcastButton')}
        </button>
        <button class="mi" onclick={() => (showRecordings = true)}>
          <Icon name="film" />{t('recordingsButton')}
        </button>
      </Menu>

      <Menu label={t('groupRestrictions')}>
        {#snippet icon()}<Icon name="shield" />{/snippet}
        <button class="mi" onclick={() => (settingWallpaper = true)} disabled={devices.length === 0}>
          <Icon name="image" />{t('wallpaperButton')}…
        </button>
        <button class="mi" onclick={() => wallpaperPolicy('lock-wallpaper')} disabled={!watching}>
          <Icon name="lock" />{t('actLockWallpaper')}
        </button>
        <button class="mi" onclick={() => wallpaperPolicy('unlock-wallpaper')} disabled={!watching}>
          <Icon name="image" />{t('actUnlockWallpaper')}
        </button>
        <span class="sep"></span>
        <button class="mi" onclick={() => (editingBlocklist = true)}>
          <Icon name="ban" />{t('blockButton')}…
        </button>
        <span class="sep"></span>
        <button class="mi" onclick={() => screenLockAll(true)} disabled={!watching}>
          <Icon name="freeze" />{t('screenLockAll')}
        </button>
        <button class="mi" onclick={() => screenLockAll(false)} disabled={!watching}>
          <Icon name="freeze" />{t('screenLockStopAll')}
        </button>
        <button class="mi" onclick={() => examAll(true)} disabled={!watching}>
          <Icon name="lock" />{t('examAll')}
        </button>
        <button class="mi" onclick={() => examAll(false)} disabled={!watching}>
          <Icon name="lock" />{t('examStopAll')}
        </button>
      </Menu>

      <ActionMenu deviceId={null} liveCount={live} onerror={(m) => (error = m)} />

      <button class="icononly" onclick={() => (showSettings = true)} title={t('settings')} aria-label={t('settings')}>
        <Icon name="settings" />
      </button>
      <button class="icononly" onclick={() => (showTutorial = true)} title={t('helpHint')} aria-label={t('help')}>
        <Icon name="info" />
      </button>
    </div>
  </header>

  {#each presenting as p (p.id)}
    <div class="presenting" role="status" transition:slide={{ duration: 160, easing: cubicOut }}>
      <span class="pulse"></span>
      <span class="ptext">{t('broadcastBanner', p.label, p.targets)}</span>
      <span class="pspace"></span>
      <button class="pstop" onclick={() => stopPresenting(p.id)}>{t('broadcastStop')}</button>
    </div>
  {/each}

  {#if error}
    <div class="banner" role="alert">
      <span>{error}</span>
      <button onclick={() => (error = null)}>{t('dismiss')}</button>
    </div>
  {/if}

  <main oncontextmenu={openBgMenu}>
    {#if !loaded}
      <p class="placeholder">{t('loading')}</p>
    {:else if devices.length === 0}
      <div class="empty">
        <h2>{t('emptyTitle')}</h2>
        <p>{t('emptyBody')}</p>
        <button class="primary" onclick={() => (pairing = true)}>{t('addPc')}</button>
      </div>
    {:else}
      <div class="board" style="--tile-min: {tileMin}px">
        {#each groups.groups as group (group.id)}
          <section
            class="group"
            class:droptarget={draggingId && dropTarget?.groupId === group.id}
            animate:flip={{ duration: 200, easing: cubicOut }}
            ondragover={(e) => onGroupDragOver(e, group.id)}
            ondrop={onDrop}
            role="group"
          >
            <header class="ghead">
              <button
                class="chev"
                onclick={() => groups.toggle(group.id)}
                aria-label={group.collapsed ? t('expand') : t('collapse')}
              >
                {group.collapsed ? '▸' : '▾'}
              </button>
              {#if editingGroup === group.id}
                <input
                  class="gname-edit"
                  value={group.name}
                  use:focusInput
                  onblur={(e) => {
                    groups.rename(group.id, e.currentTarget.value)
                    editingGroup = null
                  }}
                  onkeydown={(e) => {
                    if (e.key === 'Enter') e.currentTarget.blur()
                    else if (e.key === 'Escape') editingGroup = null
                  }}
                />
              {:else}
                <button
                  class="gname"
                  ondblclick={() => group.id !== UNCAT && (editingGroup = group.id)}
                  title={group.id === UNCAT ? '' : t('renameGroupHint')}
                >
                  {group.name}
                </button>
              {/if}
              <span class="gcount">{tilesOf(group.id).length}</span>
              <span class="gspace"></span>
              <button class="dots" onclick={(e) => openGroupMenu(e, group.id)} aria-label={t('groupMenu')}
                >⋯</button
              >
            </header>

            {#if !group.collapsed}
              <div class="grid" transition:slide={{ duration: 180, easing: cubicOut }}>
                {#each tilesOf(group.id) as device, index (device.device_id)}
                  <div
                    class="cell"
                    class:dragging={draggingId === device.device_id}
                    class:dropbefore={draggingId &&
                      dropTarget?.groupId === group.id &&
                      dropTarget?.index === index}
                    animate:flip={{ duration: 200, easing: cubicOut }}
                    ondragover={(e) => onCellDragOver(e, group.id, index)}
                    ondrop={onDrop}
                    oncontextmenu={(e) => openTileMenu(e, device.device_id)}
                    role="presentation"
                  >
                    <button
                      class="grip"
                      draggable="true"
                      ondragstart={(e) => dragStart(e, device.device_id)}
                      ondragend={dragEnd}
                      title={t('dragHint')}
                      aria-label={t('dragHint')}>⠿</button
                    >
                    <button
                      class="dots tiledots"
                      onclick={(e) => openTileMenu(e, device.device_id)}
                      aria-label={t('moveTo')}>⋯</button
                    >
                    <DeviceTile
                      {device}
                      {watching}
                      {keepPreviews}
                      onopen={() => open(device.device_id)}
                      onmonitor={(i) => chooseMonitor(device.device_id, i)}
                      onwake={() => wake(device.device_id)}
                      onrename={(name) => rename(device.device_id, name)}
                    />
                  </div>
                {/each}
                {#if tilesOf(group.id).length === 0}
                  <p class="gempty">{t('groupEmpty')}</p>
                {/if}
              </div>
            {/if}
          </section>
        {/each}
        <button class="addgroup" onclick={createGroup}>+ {t('newGroup')}</button>
      </div>
    {/if}
  </main>

  <footer>
    {#if info}
      <span class="whoami">
        {t('thisConsole')} <code>{info.device_id}</code>
        <code class="key" title={info.public_key}>{info.public_key}</code>
        <button class="copykey" onclick={copyKey}>{keyCopied ? t('copied') : t('copyKey')}</button>
      </span>
    {/if}
    <RoomCard onerror={(m) => (error = m)} />
    <span class="right">
      <label class="tilezoom" title={t('tileSizeHint')}>
        <span>{t('tileSize')}</span>
        <input
          type="range"
          min="180"
          max="640"
          step="20"
          value={tileMin}
          oninput={(e) => setTileMin(Number((e.currentTarget as HTMLInputElement).value))}
        />
      </label>
      <QualityPicker />
      <span class="dot-label">
        <i class="dot" class:live={watching}></i>
        {watching ? t('capturing') : t('notCapturing')}
      </span>
      <LanguagePicker />
    </span>
  </footer>
</div>
{/if}

{#if tileMenu || groupMenu || bgMenu}
  <!-- Invisible scrim: a click anywhere else closes the menu. -->
  <div
    class="menuscrim"
    role="presentation"
    onclick={closeMenus}
    oncontextmenu={(e) => (e.preventDefault(), closeMenus())}
  ></div>
{/if}

{#if bgMenu}
  <!-- Right-click-anywhere menu of class-wide actions, including a manual reload of the PC list. -->
  <div class="popover" style="left:{bgMenu.x}px; top:{bgMenu.y}px" transition:fade={{ duration: 90 }}>
    <p class="pop-label">{t('menuTitle')}</p>
    <button class="pop-item" onclick={() => (refresh(), closeMenus())}>{t('menuReload')}</button>
    <button
      class="pop-item"
      onclick={() => (toggleWatching(), closeMenus())}
      disabled={devices.length === 0}
    >
      {watching ? t('stopWatching') : t('startWatching')}
    </button>
    <button class="pop-item" onclick={() => ((pairing = true), closeMenus())}>{t('addPc')}</button>
    <button
      class="pop-item"
      onclick={() => ((broadcasting = true), closeMenus())}
      disabled={devices.length === 0}
    >
      {t('broadcastButton')}
    </button>
    <button class="pop-item" onclick={() => ((showSettings = true), closeMenus())}>{t('settings')}</button>
  </div>
{/if}

{#if tileMenu}
  {@const currentGroup = groups.groupOf(tileMenu.id)}
  <div class="popover" style="left:{tileMenu.x}px; top:{tileMenu.y}px" transition:fade={{ duration: 90 }}>
    <p class="pop-label">{t('moveTo')}</p>
    {#each groups.groups as g (g.id)}
      <button
        class="pop-item"
        class:current={g.id === currentGroup}
        onclick={() => moveToGroup(tileMenu?.id ?? '', g.id)}
      >
        {g.name}
      </button>
    {/each}
    <button class="pop-item new" onclick={() => moveToNewGroup(tileMenu?.id ?? '')}
      >+ {t('newGroup')}</button
    >
  </div>
{/if}

{#if groupMenu}
  <div class="popover" style="left:{groupMenu.x}px; top:{groupMenu.y}px" transition:fade={{ duration: 90 }}>
    {#if groupMenu.id !== UNCAT}
      <button
        class="pop-item"
        onclick={() => {
          editingGroup = groupMenu?.id ?? null
          closeMenus()
        }}>{t('rename')}</button
      >
    {/if}
    <button
      class="pop-item"
      onclick={() => {
        groups.toggle(groupMenu?.id ?? '')
        closeMenus()
      }}>{groups.groups.find((g) => g.id === groupMenu?.id)?.collapsed ? t('expand') : t('collapse')}</button
    >
    {#if groupMenu.id !== UNCAT}
      <button
        class="pop-item danger"
        onclick={() => {
          groups.remove(groupMenu?.id ?? '')
          closeMenus()
        }}>{t('deleteGroup')}</button
      >
    {/if}
  </div>
{/if}

{#if pairing}
  <PairDialog
    onclose={() => {
      pairing = false
      refresh()
    }}
  />
{/if}

{#if showTutorial}
  <Tutorial onclose={() => (showTutorial = false)} />
{/if}

{#if showSettings}
  <SettingsDialog
    {keepPreviews}
    onKeepPreviews={setKeepPreviews}
    {settings}
    onSettings={saveSettings}
    onclose={() => (showSettings = false)}
  />
{/if}

{#if editingBlocklist}
  <BlocklistDialog onclose={() => (editingBlocklist = false)} />
{/if}

{#if creatingClassroom}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && (creatingClassroom = false)}>
    <div class="mini" role="dialog" aria-modal="true" aria-label={t('classroomNew')}>
      <h2>{t('classroomNew')}</h2>
      <p class="hint">{t('classroomNewHint')}</p>
      <input
        type="text"
        bind:value={newClassroomName}
        placeholder={t('classroomNamePlaceholder')}
        onkeydown={(e) => e.key === 'Enter' && submitNewClassroom()}
      />
      <div class="row">
        <button onclick={() => (creatingClassroom = false)}>{t('cancel')}</button>
        <button class="primary" disabled={!newClassroomName.trim()} onclick={submitNewClassroom}>
          {t('classroomCreateOpen')}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if broadcasting}
  <BroadcastDialog
    {devices}
    onclose={() => (broadcasting = false)}
    onerror={(m) => (error = m)}
  />
{/if}

{#if showRecordings}
  <RecordingsDialog onclose={() => (showRecordings = false)} onerror={(m) => (error = m)} />
{/if}

{#if settingWallpaper}
  <WallpaperDialog
    {devices}
    onclose={() => (settingWallpaper = false)}
    onerror={(m) => (error = m)}
  />
{/if}

<!-- Surey (AI) appears only when AI features are enabled in Settings. -->
{#if settings.ai_enabled}
  {#if sureyOpen}
    <SureyPanel onclose={() => (sureyOpen = false)} />
  {:else}
    <SureyLauncher onopen={() => (sureyOpen = true)} />
  {/if}
{/if}

{#if focusedDevice}
  <Focused
    device={focusedDevice}
    onclose={() => open(null)}
    onmonitor={(index) => chooseMonitor(focusedDevice.device_id, index)}
    listening={listeningTo === focusedDevice.device_id}
    onlisten={(on) => listen(on ? focusedDevice.device_id : null)}
    controlling={controllingId === focusedDevice.device_id}
    oncontrol={(on) => control(on ? focusedDevice.device_id : null)}
    onerror={(m) => (error = m)}
  />
{/if}

<Toasts />

<style>
  /* Small centred modal (new-classroom prompt). */
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.5);
  }
  .mini {
    width: min(420px, 92vw);
    padding: 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 12px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
  }
  .mini h2 {
    margin: 0 0 6px;
    font-size: 17px;
  }
  .mini .hint {
    margin: 0 0 14px;
    color: var(--muted);
    font-size: 13px;
  }
  .mini input {
    width: 100%;
    padding: 10px 12px;
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font: inherit;
  }
  .mini .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }

  /* A flex column, not a fixed grid: the footer stays pinned to the bottom whether or not the error
     banner is showing. (The old 4-row grid mis-placed the footer into a tall row when the banner was
     absent, floating it into the middle of the screen.) */
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 16px;
    padding: 16px 20px;
    border-bottom: 1px solid var(--line);
    background: var(--panel);
  }

  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }

  .sub {
    margin: 2px 0 0;
    color: var(--muted);
    font-size: 13px;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  /* Buttons that pair an icon with their label, and icon-only buttons (settings, help). */
  .actions :global(.withicon) {
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }

  .actions :global(.icononly) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    padding: 0;
    color: var(--muted);
  }

  .actions :global(.icononly:hover) {
    color: var(--text);
    border-color: var(--accent, var(--line));
  }

  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 20px;
    background: #3a1f1f;
    border-bottom: 1px solid #5a2b2b;
    color: #ffd7d7;
    font-size: 13px;
  }

  /* Presenting banner: quiet, accent-tinted, always visible while a broadcast runs in the background. */
  .presenting {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 20px;
    background: color-mix(in srgb, var(--accent) 16%, var(--panel));
    border-bottom: 1px solid color-mix(in srgb, var(--accent) 40%, var(--line));
    color: var(--text);
    font-size: 13px;
  }

  .presenting .pulse {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 70%, transparent);
    animation: presenting-pulse 1.8s ease-out infinite;
  }

  @keyframes presenting-pulse {
    0% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 60%, transparent);
    }
    70% {
      box-shadow: 0 0 0 7px transparent;
    }
    100% {
      box-shadow: 0 0 0 0 transparent;
    }
  }

  .presenting .ptext {
    font-weight: 500;
  }

  .presenting .pspace {
    flex: 1;
  }

  .presenting .pstop {
    padding: 4px 12px;
    font-size: 12.5px;
    font-weight: 600;
    background: var(--accent);
    border: 1px solid var(--accent);
    border-radius: 8px;
    color: #06101f;
    cursor: pointer;
  }

  main {
    flex: 1 1 auto;
    min-height: 0;
    overflow: auto;
    padding: 18px 20px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--tile-min, 260px), 1fr));
    gap: 14px;
  }

  /* Groups: spacing and a light header do the grouping, not heavy cards. */
  .board {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .ghead {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-bottom: 2px;
    border-bottom: 1px solid var(--line);
  }

  .chev {
    width: 22px;
    padding: 2px;
    background: transparent;
    border: 0;
    color: var(--muted);
    font-size: 11px;
    cursor: pointer;
  }

  .gname {
    padding: 2px 4px;
    background: transparent;
    border: 0;
    border-radius: 6px;
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
    cursor: default;
  }

  .gname-edit {
    padding: 3px 7px;
    background: var(--bg);
    border: 1px solid var(--accent);
    border-radius: 6px;
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
  }

  .gcount {
    color: var(--muted);
    font-size: 12px;
  }

  .gspace {
    flex: 1;
  }

  .dots {
    width: 26px;
    padding: 2px;
    background: transparent;
    border: 0;
    border-radius: 6px;
    color: var(--muted);
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
  }

  .dots:hover {
    background: var(--bg);
    color: var(--text);
  }

  .cell {
    position: relative;
  }

  .cell.dragging {
    opacity: 0.35;
  }

  /* Drop hints while dragging: a bar before the target tile, a soft ring around the target group. */
  .cell.dropbefore {
    position: relative;
  }

  .cell.dropbefore::before {
    content: '';
    position: absolute;
    inset: -7px auto -7px -8px;
    width: 3px;
    border-radius: 3px;
    background: var(--accent);
    z-index: 4;
  }

  .group.droptarget {
    outline: 2px dashed color-mix(in srgb, var(--accent) 55%, transparent);
    outline-offset: 6px;
    border-radius: 10px;
  }

  .grip,
  .tiledots {
    position: absolute;
    top: 6px;
    z-index: 3;
    width: 24px;
    height: 24px;
    padding: 0;
    background: rgba(8, 11, 16, 0.7);
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--muted);
    font-size: 13px;
    line-height: 1;
    opacity: 0;
    transition: opacity 0.12s;
  }

  .grip {
    left: 6px;
    cursor: grab;
  }

  .grip:active {
    cursor: grabbing;
  }

  .tiledots {
    right: 6px;
  }

  .cell:hover .grip,
  .cell:hover .tiledots {
    opacity: 0.85;
  }

  .gempty {
    grid-column: 1 / -1;
    margin: 0;
    padding: 10px 4px;
    color: var(--muted);
    font-size: 12.5px;
  }

  .addgroup {
    align-self: flex-start;
    padding: 6px 12px;
    font-size: 12.5px;
    color: var(--muted);
    border-style: dashed;
  }

  .addgroup:hover {
    color: var(--text);
    border-color: var(--accent);
  }

  .menuscrim {
    position: fixed;
    inset: 0;
    z-index: 55;
  }

  .popover {
    position: fixed;
    z-index: 60;
    min-width: 176px;
    max-width: 240px;
    padding: 6px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 10px;
    box-shadow: 0 14px 36px rgba(0, 0, 0, 0.5);
  }

  .pop-label {
    margin: 2px 6px 4px;
    color: var(--muted);
    font-size: 11px;
  }

  .pop-item {
    display: block;
    width: 100%;
    padding: 6px 8px;
    text-align: left;
    background: transparent;
    border: 0;
    border-radius: 6px;
    color: var(--text);
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pop-item:hover {
    background: var(--bg);
  }

  .pop-item.current {
    color: var(--accent);
    font-weight: 600;
  }

  .pop-item.new {
    margin-top: 4px;
    border-top: 1px solid var(--line);
    color: var(--muted);
  }

  .pop-item.danger:hover {
    color: var(--danger);
  }

  .empty,
  .placeholder {
    max-width: 420px;
    margin: 12vh auto 0;
    text-align: center;
    color: var(--muted);
  }

  .empty h2 {
    margin: 0 0 6px;
    color: var(--text);
    font-size: 17px;
  }

  .empty p {
    margin: 0 0 18px;
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    flex-shrink: 0;
    gap: 8px 16px;
    padding: 9px 20px;
    border-top: 1px solid var(--line);
    background: var(--panel);
    color: var(--muted);
    font-size: 12px;
  }

  .whoami {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  /* The full 64-char endpoint key is what a student PC needs to dial this console; show it, but let
     it shrink with an ellipsis so it never pushes the room card off the footer. */
  .whoami .key {
    max-width: 22ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
  }

  .copykey {
    padding: 2px 8px;
    font-size: 11px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--text);
    cursor: pointer;
  }

  .copykey:hover {
    border-color: var(--accent, var(--line));
  }

  .boot {
    height: 100%;
    background: var(--bg);
  }

  .right {
    display: inline-flex;
    align-items: center;
    gap: 14px;
  }

  .dot-label {
    display: inline-flex;
    align-items: center;
    gap: 7px;
  }

  .tilezoom {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
    font-size: 12px;
  }

  .tilezoom input {
    width: 96px;
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #48515e;
  }

  .dot.live {
    background: var(--live);
    box-shadow: 0 0 0 3px rgba(62, 207, 142, 0.18);
  }

</style>
