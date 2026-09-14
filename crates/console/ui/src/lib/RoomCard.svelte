<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { t } from './i18n.svelte'

  /**
   * The room this console owns: its name, and the password a PC needs in order to *leave*.
   *
   * The password is shown on request rather than always, because it goes on the projector during a
   * lesson. It exists only on this machine (sealed with DPAPI); student PCs only ever get its hash.
   */
  let { onerror }: { onerror: (message: string) => void } = $props()

  interface RoomInfo {
    name: string
    password: string
  }

  let room = $state<RoomInfo | null>(null)
  let open = $state(false)
  let revealed = $state(false)
  let draft = $state('')

  async function load() {
    try {
      room = await invoke<RoomInfo>('room_info')
      draft = room.name
    } catch (e) {
      onerror(String(e))
    }
  }
  load()

  async function rename() {
    try {
      await invoke('rename_room', { name: draft })
      await load()
    } catch (e) {
      onerror(String(e))
    }
  }

  async function regenerate() {
    try {
      await invoke('new_room_password')
      await load()
      revealed = true
    } catch (e) {
      onerror(String(e))
    }
  }
</script>

<span class="wrap">
  <button class="link" onclick={() => (open = !open)}>
    {t('roomLabel')} <strong>{room?.name ?? '—'}</strong>
  </button>

  {#if open && room}
    <div class="panel" role="dialog" aria-label={t('roomLabel')}>
      <p class="label">{t('roomName')}</p>
      <div class="row">
        <input bind:value={draft} aria-label={t('roomName')} />
        <button onclick={rename} disabled={draft.trim() === room.name}>{t('roomSave')}</button>
      </div>

      <p class="label">{t('roomPassword')}</p>
      <p class="hint">{t('roomPasswordHint')}</p>
      <div class="row">
        {#if revealed}
          <code class="password">{room.password}</code>
        {:else}
          <code class="password hidden">••••-••••-••••</code>
        {/if}
        <button onclick={() => (revealed = !revealed)}>
          {revealed ? t('roomHide') : t('roomShow')}
        </button>
      </div>
      <button class="regen" onclick={regenerate}>{t('roomNewPassword')}</button>
    </div>
  {/if}
</span>

<style>
  .wrap {
    position: relative;
    display: inline-flex;
  }

  .link {
    padding: 2px 6px;
    background: transparent;
    border-color: transparent;
    color: var(--muted);
    font-size: 12px;
  }

  .link:hover {
    color: var(--text);
    border-color: var(--line);
  }

  .panel {
    position: absolute;
    bottom: calc(100% + 8px);
    right: 0;
    z-index: 25;
    width: 290px;
    padding: 12px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: var(--radius);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
  }

  .label {
    margin: 0 0 4px;
    color: var(--muted);
    font-size: 11.5px;
  }

  .hint {
    margin: 0 0 6px;
    color: var(--muted);
    font-size: 11.5px;
    line-height: 1.45;
  }

  .row {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-bottom: 10px;
  }

  input {
    flex: 1;
    min-width: 0;
    padding: 6px 8px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font-size: 12.5px;
  }

  .password {
    flex: 1;
    padding: 6px 8px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 12.5px;
    letter-spacing: 0.5px;
  }

  .password.hidden {
    color: var(--muted);
  }

  .regen {
    width: 100%;
    font-size: 12px;
  }
</style>
