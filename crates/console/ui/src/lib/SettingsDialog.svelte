<script lang="ts">
  import { t } from './i18n.svelte'

  /**
   * Console settings + an "about" section. Kept small and local: the only real setting so far is
   * whether previews linger when you stop watching (or a PC goes offline). The about block credits the
   * author and acknowledges the main open-source dependencies.
   */
  let {
    keepPreviews,
    onKeepPreviews,
    onclose,
  }: {
    keepPreviews: boolean
    onKeepPreviews: (value: boolean) => void
    onclose: () => void
  } = $props()

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
