<script lang="ts">
  import { i18n, t } from './i18n.svelte'

  let note = $state<string | null>(null)

  async function onchange(event: Event) {
    const code = (event.currentTarget as HTMLSelectElement).value
    if (code === '__template__') {
      // Reset the select back to the current language before doing the export.
      ;(event.currentTarget as HTMLSelectElement).value = i18n.code
      try {
        const path = await i18n.exportTemplate()
        note = t('templateWritten', path)
      } catch (e) {
        note = t('templateFailed', String(e))
      }
      return
    }
    await i18n.choose(code)
  }
</script>

<label class="picker">
  <span class="sr-only">{t('language')}</span>
  <select value={i18n.code} {onchange} aria-label={t('language')} title={t('language')}>
    {#each i18n.available as option (option.code)}
      <option value={option.code}>{option.name}</option>
    {/each}
    <option value="__template__">{t('newTranslation')}</option>
  </select>
</label>

{#if note}
  <div class="note" role="status">
    <span>{note}</span>
    <button onclick={() => (note = null)}>{t('dismiss')}</button>
  </div>
{/if}

<style>
  .picker select {
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 5px 8px;
    font: inherit;
    font-size: 12px;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  .note {
    position: fixed;
    left: 50%;
    bottom: 46px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: min(680px, 92vw);
    padding: 10px 14px;
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 10px;
    font-size: 12.5px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.45);
  }

  .note span {
    word-break: break-all;
  }
</style>
