<script lang="ts">
  import { fly, fade } from 'svelte/transition'
  import { flip } from 'svelte/animate'
  import { toasts } from './toast-store.svelte'
</script>

<div class="stack" aria-live="polite">
  {#each toasts.items as toast (toast.id)}
    <div
      class="toast {toast.kind}"
      role="status"
      in:fly={{ x: 24, duration: 180 }}
      out:fade={{ duration: 140 }}
      animate:flip={{ duration: 160 }}
    >
      <i class="dot"></i>
      <span class="msg">{toast.message}</span>
    </div>
  {/each}
</div>

<style>
  .stack {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 80;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-end;
    pointer-events: none;
  }

  .toast {
    pointer-events: auto;
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: 320px;
    padding: 9px 12px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-left-width: 3px;
    border-radius: 10px;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.45);
    color: var(--text);
    font-size: 12.5px;
    cursor: pointer;
  }

  .toast.ok {
    border-left-color: var(--live);
  }
  .toast.error {
    border-left-color: var(--danger);
  }
  .toast.info {
    border-left-color: var(--accent);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--accent);
  }
  .toast.ok .dot {
    background: var(--live);
  }
  .toast.error .dot {
    background: var(--danger);
  }

  .msg {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
