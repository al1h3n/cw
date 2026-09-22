<script lang="ts">
  import { invoke } from './bridge'
  import { onMount } from 'svelte'
  import { t } from './i18n.svelte'
  import type { PreviewWidths } from './types'

  // Width in pixels the agent is asked to scale to. Bigger is sharper and uses more bandwidth;
  // the label says what it costs so the choice is informed rather than a mystery number.
  const GRID = [
    { width: 240, key: 'qualityLow' },
    { width: 480, key: 'qualityNormal' },
    { width: 720, key: 'qualityHigh' },
  ]
  const FOCUSED = [
    { width: 720, key: 'qualityHigh' },
    { width: 1280, key: 'qualityFull' },
    { width: 1920, key: 'qualityNative' },
  ]

  // Compression quality is separate from size: same pixels, more or fewer JPEG artefacts. A 1080p
  // screen can still look blocky at low quality, which is what this lets a teacher fix.
  const COMPRESSION = [
    { q: 45, key: 'compressLight' },
    { q: 60, key: 'compressBalanced' },
    { q: 80, key: 'compressSharp' },
    { q: 92, key: 'compressMax' },
  ]

  let widths = $state<PreviewWidths>({ grid: 480, focused: 1280, quality: 60 })

  onMount(async () => {
    widths = await invoke<PreviewWidths>('preview_widths')
  })

  async function apply(next: PreviewWidths) {
    widths = next
    await invoke('set_preview_widths', {
      grid: next.grid,
      focused: next.focused,
      quality: next.quality,
    })
  }
</script>

<label class="quality">
  <span>{t('qualityGrid')}</span>
  <select
    value={String(widths.grid)}
    onchange={(e) => apply({ ...widths, grid: Number((e.currentTarget as HTMLSelectElement).value) })}
  >
    {#each GRID as option (option.width)}
      <option value={String(option.width)}>{t(option.key)} · {option.width}px</option>
    {/each}
  </select>
</label>

<label class="quality">
  <span>{t('qualityFocused')}</span>
  <select
    value={String(widths.focused)}
    onchange={(e) =>
      apply({ ...widths, focused: Number((e.currentTarget as HTMLSelectElement).value) })}
  >
    {#each FOCUSED as option (option.width)}
      <option value={String(option.width)}>{t(option.key)} · {option.width}px</option>
    {/each}
  </select>
</label>

<label class="quality">
  <span>{t('qualityCompression')}</span>
  <select
    value={String(widths.quality)}
    onchange={(e) =>
      apply({ ...widths, quality: Number((e.currentTarget as HTMLSelectElement).value) })}
  >
    {#each COMPRESSION as option (option.q)}
      <option value={String(option.q)}>{t(option.key)}</option>
    {/each}
  </select>
</label>

<style>
  .quality {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
    font-size: 12px;
  }

  .quality select {
    background: var(--panel-2);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 5px 8px;
    font: inherit;
    font-size: 12px;
  }
</style>
