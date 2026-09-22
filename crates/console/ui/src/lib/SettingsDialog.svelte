<script lang="ts">
  import { onMount } from 'svelte'
  import { t } from './i18n.svelte'
  import { invoke } from './bridge'
  import { DEFAULT_CUSTOM } from './theme'
  import type { Settings, CloudStatus } from './types'

  /**
   * Console settings + an "about" section: preview persistence, an AI on/off switch, the colour theme
   * (light / dark / a custom palette), and a cloud-sync card for the paid dashboard. The about block
   * credits the author and acknowledges the main open-source dependencies.
   */
  let {
    keepPreviews,
    onKeepPreviews,
    settings,
    onSettings,
    onclose,
  }: {
    keepPreviews: boolean
    onKeepPreviews: (value: boolean) => void
    settings: Settings
    onSettings: (value: Settings) => void
    onclose: () => void
  } = $props()

  // A local editable copy; every change is pushed back through `onSettings`, which saves + re-themes.
  let s = $state<Settings>({
    ...settings,
    custom: { ...DEFAULT_CUSTOM, ...settings.custom },
  })
  const COLORS: { key: keyof Settings['custom']; label: string }[] = [
    { key: 'bg', label: 'themeBg' },
    { key: 'panel', label: 'themePanel' },
    { key: 'accent', label: 'themeAccent' },
    { key: 'text', label: 'themeText' },
  ]
  function commit() {
    onSettings($state.snapshot(s))
  }

  // Cloud sync (paid dashboard). The endpoint + licence live in the subscription store.
  let dashboardUrl = $state('')
  let license = $state('')
  let cloud = $state<CloudStatus | null>(null)
  let cloudBusy = $state(false)
  let cloudMsg = $state('')

  onMount(async () => {
    try {
      const sub = await invoke<{ dashboard_url: string }>('subscription_config')
      dashboardUrl = sub.dashboard_url
      cloud = await invoke<CloudStatus>('cloud_status')
    } catch {
      // A missing subscription store just leaves the card at its defaults.
    }
  })

  async function saveCloud() {
    try {
      await invoke('subscription_set', { dashboardUrl, license: license || null })
      license = ''
      cloud = await invoke<CloudStatus>('cloud_status')
      cloudMsg = t('cloudSaved')
    } catch (e) {
      cloudMsg = String(e)
    }
  }

  async function syncNow() {
    cloudBusy = true
    cloudMsg = ''
    try {
      await invoke('cloud_push')
      cloudMsg = t('cloudSynced')
    } catch (e) {
      cloudMsg = String(e)
    } finally {
      cloudBusy = false
    }
  }

  // Libraries the app is built on, credited here (many are permissive licences that ask for
  // acknowledgement). Shown quietly so the section informs without shouting.
  const CREDITS = [
    'iroh — QUIC transport, NAT traversal (Apache-2.0/MIT)',
    'Tauri + Svelte — desktop shell and UI (Apache-2.0/MIT)',
    'the windows crate — official Win32 bindings (Apache-2.0/MIT)',
    'OpenH264 — H.264 fallback encoder (BSD-2-Clause, Cisco)',
    'zune-jpeg, argon2, postcard, tokio, serde (Apache-2.0/MIT)',
    'FFmpeg — optional, for recorded video (LGPL/GPL, if present)',
  ]

  function onkey(event: KeyboardEvent) {
    if (event.key === 'Escape') onclose()
  }
</script>

<svelte:window on:keydown={onkey} />

<div class="backdrop" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={t('settingsTitle')}>
    <h2>{t('settingsTitle')}</h2>

    <section>
      <h3>{t('settingsPreviews')}</h3>
      <label class="toggle">
        <input
          type="checkbox"
          checked={keepPreviews}
          onchange={(e) => onKeepPreviews((e.currentTarget as HTMLInputElement).checked)}
        />
        <span>
          <strong>{t('settingsKeepPreviews')}</strong>
          <span class="hint">{t('settingsKeepPreviewsHint')}</span>
        </span>
      </label>
    </section>

    <section>
      <h3>{t('settingsAi')}</h3>
      <label class="toggle">
        <input
          type="checkbox"
          checked={s.ai_enabled}
          onchange={(e) => {
            s.ai_enabled = (e.currentTarget as HTMLInputElement).checked
            commit()
          }}
        />
        <span>
          <strong>{t('settingsAiEnabled')}</strong>
          <span class="hint">{t('settingsAiEnabledHint')}</span>
        </span>
      </label>
    </section>

    <section>
      <h3>{t('settingsTheme')}</h3>
      <div class="themes">
        {#each ['dark', 'light', 'custom'] as const as opt (opt)}
          <button
            class="theme-chip"
            class:on={s.theme === opt}
            onclick={() => {
              s.theme = opt
              commit()
            }}
          >
            {t('theme_' + opt)}
          </button>
        {/each}
      </div>
      {#if s.theme === 'custom'}
        <div class="colors">
          {#each COLORS as c (c.key)}
            <label class="color">
              <input
                type="color"
                value={s.custom[c.key]}
                onchange={(e) => {
                  s.custom[c.key] = (e.currentTarget as HTMLInputElement).value
                  commit()
                }}
              />
              <span>{t(c.label)}</span>
            </label>
          {/each}
        </div>
      {/if}
    </section>

    <section>
      <h3>{t('settingsCloud')}</h3>
      <p class="hint">{t('settingsCloudHint')}</p>
      <label class="field">
        <span>{t('cloudDashboardUrl')}</span>
        <input type="url" bind:value={dashboardUrl} placeholder="https://dashboard.example/team" />
      </label>
      <label class="field">
        <span>{t('cloudLicense')}</span>
        <input
          type="password"
          bind:value={license}
          placeholder={cloud?.configured ? '••••••••' : t('cloudLicensePlaceholder')}
        />
      </label>
      <div class="cloud-row">
        <button onclick={saveCloud}>{t('save')}</button>
        <button onclick={syncNow} disabled={cloudBusy || !cloud?.configured}>
          {cloudBusy ? t('cloudSyncing') : t('cloudSyncNow')}
        </button>
        <span class="cloud-status">{cloudMsg || cloud?.detail || ''}</span>
      </div>
    </section>

    <section class="about">
      <h3>{t('settingsAbout')}</h3>
      <p class="appname">{t('room')}</p>
      <p class="desc">{t('aboutDescription')}</p>
      <p class="author">{t('aboutAuthor')} <strong>Alikhan Aitugan</strong></p>
      <p class="credits-head">{t('aboutCredits')}</p>
      <ul class="credits">
        {#each CREDITS as line (line)}
          <li>{line}</li>
        {/each}
      </ul>
    </section>

    <footer>
      <span class="spacer"></span>
      <button onclick={onclose}>{t('close')}</button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    background: rgba(6, 9, 13, 0.82);
    padding: 18px;
  }

  .dialog {
    display: flex;
    flex-direction: column;
    width: min(560px, 96vw);
    max-height: 92vh;
    overflow: auto;
    padding: 22px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 14px;
  }

  h2 {
    margin: 0 0 12px;
    font-size: 17px;
  }

  h3 {
    margin: 0 0 8px;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  section {
    padding: 14px 0;
    border-top: 1px solid var(--line);
  }

  .toggle {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    font-size: 13px;
  }

  .toggle span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .toggle .hint {
    color: var(--muted);
    font-size: 12px;
  }

  .appname {
    margin: 0 0 2px;
    font-size: 16px;
    font-weight: 600;
  }

  .desc {
    margin: 0 0 8px;
    color: var(--muted);
    font-size: 12.5px;
    line-height: 1.5;
  }

  .author {
    margin: 0 0 12px;
    font-size: 13px;
  }

  /* Acknowledgements: present but quiet — semi-transparent, small, as requested. */
  .credits-head {
    margin: 0 0 4px;
    font-size: 11px;
    color: var(--muted);
    opacity: 0.6;
  }

  .credits {
    margin: 0;
    padding-left: 16px;
    list-style: disc;
    color: var(--muted);
    font-size: 11px;
    line-height: 1.6;
    opacity: 0.45;
  }

  .themes {
    display: flex;
    gap: 8px;
  }

  .theme-chip {
    flex: 1;
    padding: 8px 10px;
    font-size: 13px;
    text-transform: capitalize;
  }

  .theme-chip.on {
    background: var(--accent);
    border-color: var(--accent);
    color: #06101f;
    font-weight: 600;
  }

  .colors {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 10px;
    margin-top: 12px;
  }

  .color {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    color: var(--muted);
  }

  .color input[type='color'] {
    width: 34px;
    height: 26px;
    padding: 0;
    background: none;
    border: 1px solid var(--line);
    border-radius: 6px;
    cursor: pointer;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 10px;
    font-size: 12.5px;
    color: var(--muted);
  }

  .field input {
    padding: 8px 10px;
    background: var(--panel-2);
    border: 1px solid var(--line);
    border-radius: 8px;
    color: var(--text);
    font: inherit;
  }

  .cloud-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    flex-wrap: wrap;
  }

  .cloud-status {
    color: var(--muted);
    font-size: 12px;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 16px;
    margin-top: 4px;
  }

  .spacer {
    flex: 1;
  }
</style>
