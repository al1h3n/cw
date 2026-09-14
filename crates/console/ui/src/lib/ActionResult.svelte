<script lang="ts">
  import { t } from './i18n.svelte'
  import type { ActionReport } from './types'

  /** How long an answer stays visible. Long enough to glance at, short enough not to go stale. */
  const VISIBLE_MS = 15_000

  let { report }: { report: ActionReport | null } = $props()

  let now = $state(Date.now())
  $effect(() => {
    const timer = window.setInterval(() => (now = Date.now()), 1000)
    return () => window.clearInterval(timer)
  })

  const visible = $derived(report !== null && now - report.at_ms < VISIBLE_MS)
  const ok = $derived(report?.result === 'started')
  const text = $derived.by(() => {
    if (!report) return ''
    const action = t(`action_${report.action}`)
    if (!ok) return t(`result_${report.result}`, action)
    return report.delay_seconds > 0 ? t('result_startedIn', action, report.delay_seconds) : t('result_started', action)
  })
</script>

{#if visible}
  <span class="result" class:ok role="status">{text}</span>
{/if}

<style>
  .result {
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11.5px;
    white-space: nowrap;
    background: rgba(255, 93, 93, 0.14);
    color: var(--danger);
  }

  .result.ok {
    background: rgba(62, 207, 142, 0.14);
    color: var(--live);
  }
</style>
