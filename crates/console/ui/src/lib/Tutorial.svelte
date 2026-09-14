<script lang="ts">
  import { t } from './i18n.svelte'

  /**
   * The first-run tour. Five short steps, skippable from every one of them, and re-openable from
   * the Help button — so it is never in the way, and never lost either.
   *
   * "Seen it" is remembered in localStorage rather than on the Rust side: it is a per-teacher
   * convenience, not state anything else depends on, and it must not fail the app if it is
   * unavailable (a locked-down school profile may block it).
   */
  const SEEN_KEY = 'cowatcher.tutorial.seen'

  let { onclose }: { onclose: () => void } = $props()

  let step = $state(0)

  const steps = $derived([
    { title: t('tourAddTitle'), body: t('tourAddBody') },
    { title: t('tourWatchTitle'), body: t('tourWatchBody') },
    { title: t('tourOpenTitle'), body: t('tourOpenBody') },
    { title: t('tourControlTitle'), body: t('tourControlBody') },
    { title: t('tourRoomTitle'), body: t('tourRoomBody') },
  ])

  const isLast = $derived(step === steps.length - 1)

  function finish() {
    try {
      localStorage.setItem(SEEN_KEY, 'yes')
    } catch {
      // A profile that blocks storage just sees the tour again next time. Not worth an error.
    }
    onclose()
  }

  function onkey(event: KeyboardEvent) {
    if (event.key === 'Escape') finish()
    if (event.key === 'ArrowRight' && !isLast) step += 1
    if (event.key === 'ArrowLeft' && step > 0) step -= 1
  }
</script>

<svelte:window on:keydown={onkey} />

<div class="backdrop" role="presentation">
  <div class="card" role="dialog" aria-modal="true" aria-label={t('tourTitle')}>
    <header>
      <span class="badge">{t('tourStep', step + 1, steps.length)}</span>
      <!-- Skip is on every step, as the brief asks. -->
      <button class="skip" onclick={finish}>{t('tourSkip')}</button>
    </header>

    <h2>{steps[step].title}</h2>
    <p>{steps[step].body}</p>

    <div class="dots" role="tablist" aria-label={t('tourTitle')}>
      {#each steps as _, index (index)}
        <button
          class="dot"
          class:active={index === step}
          role="tab"
          aria-selected={index === step}
          aria-label={t('tourStep', index + 1, steps.length)}
          onclick={() => (step = index)}
        ></button>
      {/each}
    </div>

    <footer>
      <button onclick={() => (step -= 1)} disabled={step === 0}>{t('tourBack')}</button>
      {#if isLast}
        <button class="primary" onclick={finish}>{t('tourDone')}</button>
      {:else}
        <button class="primary" onclick={() => (step += 1)}>{t('tourNext')}</button>
      {/if}
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: rgba(6, 9, 13, 0.86);
    padding: 18px;
  }

  .card {
    width: min(460px, 100%);
    padding: 20px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 14px;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }

  .badge {
    color: var(--muted);
    font-size: 12px;
  }

  .skip {
    background: transparent;
    border-color: transparent;
    color: var(--muted);
    font-size: 12.5px;
  }

  .skip:hover {
    color: var(--text);
    border-color: var(--line);
  }

  h2 {
    margin: 0 0 8px;
    font-size: 17px;
  }

  p {
    margin: 0 0 16px;
    color: var(--muted);
    font-size: 13.5px;
    line-height: 1.55;
  }

  .dots {
    display: flex;
    gap: 6px;
    margin-bottom: 16px;
  }

  .dot {
    width: 22px;
    height: 4px;
    padding: 0;
    border: 0;
    border-radius: 2px;
    background: #39414d;
    cursor: pointer;
  }

  .dot.active {
    background: var(--accent);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
