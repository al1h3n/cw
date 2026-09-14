<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { t } from './i18n.svelte'

  let { onclose }: { onclose: () => void } = $props()

  /** A starter set of the games and stores the brief names, so a teacher rarely types from scratch. */
  const SUGGESTIONS = [
    'steam.exe',
    'roblox.exe',
    'robloxplayerbeta.exe',
    'epicgameslauncher.exe',
    'minecraft.exe',
    'minecraftlauncher.exe',
    'discord.exe',
  ]

  let programs = $state<string[]>([])
  let draft = $state('')
  let saving = $state(false)
  let loaded = $state(false)

  async function load() {
    programs = await invoke<string[]>('blocklist')
    loaded = true
  }
  load()

  const missing = $derived(SUGGESTIONS.filter((s) => !programs.includes(s)))

  function add(name: string) {
    const clean = name.trim().toLowerCase()
    if (clean && !programs.includes(clean)) programs = [...programs, clean]
    draft = ''
  }

  function remove(name: string) {
    programs = programs.filter((p) => p !== name)
  }

  async function save() {
    saving = true
    try {
      await invoke('set_blocklist', { programs })
      onclose()
    } catch (e) {
      alert(String(e))
    } finally {
      saving = false
    }
  }

  function onkey(event: KeyboardEvent) {
    if (event.key === 'Escape') onclose()
  }
</script>

<svelte:window on:keydown={onkey} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('blockTitle')}>
    <h2>{t('blockTitle')}</h2>
    <p class="lead">{t('blockLead')}</p>

    <form
      onsubmit={(e) => {
        e.preventDefault()
        add(draft)
      }}
    >
      <input bind:value={draft} placeholder={t('blockPlaceholder')} aria-label={t('blockPlaceholder')} />
      <button type="submit" disabled={!draft.trim()}>{t('blockAdd')}</button>
    </form>

    {#if loaded}
      {#if programs.length === 0}
        <p class="empty">{t('blockEmpty')}</p>
      {:else}
        <ul class="list">
          {#each programs as name (name)}
            <li>
              <span class="name">{name}</span>
              <button class="x" onclick={() => remove(name)} aria-label={t('blockRemove', name)}>×</button>
            </li>
          {/each}
        </ul>
      {/if}

      {#if missing.length > 0}
        <p class="hint">{t('blockSuggest')}</p>
        <div class="chips">
          {#each missing as name (name)}
            <button class="chip" onclick={() => add(name)}>+ {name}</button>
          {/each}
        </div>
      {/if}
    {/if}

    <footer>
      <span class="count">{t('blockCount', programs.length)}</span>
      <span class="spacer"></span>
      <button onclick={onclose}>{t('cancel')}</button>
      <button class="primary" onclick={save} disabled={saving}>{t('blockSave')}</button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(6, 9, 13, 0.82);
    padding: 18px;
  }

  .dialog {
    width: min(520px, 100%);
    max-height: 100%;
    overflow: auto;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 14px;
    padding: 20px;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 17px;
  }

  .lead {
    margin: 0 0 14px;
    color: var(--muted);
    font-size: 13px;
  }

  form {
    display: flex;
    gap: 8px;
    margin-bottom: 12px;
  }

  input {
    flex: 1;
    padding: 8px 10px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font-size: 13px;
  }

  .list {
    list-style: none;
    margin: 0 0 12px;
    padding: 0;
    display: grid;
    gap: 4px;
  }

  .list li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
  }

  .name {
    flex: 1;
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 13px;
  }

  .x {
    padding: 0 8px;
    font-size: 16px;
    line-height: 1;
    background: transparent;
    border-color: transparent;
    color: var(--muted);
  }

  .x:hover {
    color: var(--danger);
  }

  .empty,
  .hint {
    color: var(--muted);
    font-size: 12.5px;
    margin: 0 0 8px;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 12px;
  }

  .chip {
    padding: 4px 10px;
    font-size: 11.5px;
    border-radius: 999px;
    background: transparent;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 6px;
    border-top: 1px solid var(--line);
  }

  .count {
    color: var(--muted);
    font-size: 12px;
  }

  .spacer {
    flex: 1;
  }
</style>
