<script lang="ts">
  import { invoke } from './bridge'
  import { t } from './i18n.svelte'

  let { onclose }: { onclose: () => void } = $props()

  // Common distractions, as bare domains. Websites are OS-agnostic, so no per-OS grouping is needed.
  const SUGGESTIONS = [
    'youtube.com',
    'tiktok.com',
    'instagram.com',
    'facebook.com',
    'twitch.tv',
    'reddit.com',
    'discord.com',
    'roblox.com',
  ]

  let patterns = $state<string[]>([])
  let draft = $state('')
  let saving = $state(false)
  let loaded = $state(false)

  async function load() {
    patterns = await invoke<string[]>('web_blocklist')
    loaded = true
  }
  load()

  const missing = $derived(SUGGESTIONS.filter((s) => !patterns.includes(s)))

  function add(name: string) {
    const clean = name.trim().toLowerCase()
    if (clean && !patterns.includes(clean)) patterns = [...patterns, clean]
    draft = ''
  }
  function remove(name: string) {
    patterns = patterns.filter((p) => p !== name)
  }

  async function save() {
    saving = true
    try {
      await invoke('set_web_blocklist', { patterns })
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
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('webBlockTitle')}>
    <h2>{t('webBlockTitle')}</h2>
    <p class="lead">{t('webBlockLead')}</p>

    <form onsubmit={(e) => (e.preventDefault(), add(draft))}>
      <input bind:value={draft} placeholder={t('webBlockPlaceholder')} aria-label={t('webBlockPlaceholder')} />
      <button type="submit" disabled={!draft.trim()}>{t('blockAdd')}</button>
    </form>

    {#if loaded}
      {#if patterns.length === 0}
        <p class="empty">{t('webBlockEmpty')}</p>
      {:else}
        <ul class="list">
          {#each patterns as name (name)}
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

    <p class="note">{t('webBlockNote')}</p>

    <footer>
      <span class="count">{t('blockCount', patterns.length)}</span>
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
  .note {
    margin: 4px 0 0;
    color: var(--muted);
    font-size: 11.5px;
    opacity: 0.85;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
    margin-top: 8px;
  }
  .count {
    color: var(--muted);
    font-size: 12px;
  }
  .spacer {
    flex: 1;
  }
</style>
