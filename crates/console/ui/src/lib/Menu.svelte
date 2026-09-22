<script lang="ts">
  import type { Snippet } from 'svelte'

  /**
   * A labelled dropdown that groups related actions behind one button, so a long row of buttons
   * becomes a few tidy groups (Content, Restrictions, …). The trigger shows an icon + label; the
   * items are whatever the caller renders inside. Clicking an item, clicking away, or Escape closes it.
   */
  let {
    label,
    icon,
    children,
    align = 'left',
    disabled = false,
  }: {
    label: string
    icon?: Snippet
    children: Snippet
    align?: 'left' | 'right'
    disabled?: boolean
  } = $props()

  let open = $state(false)
  let wrap = $state<HTMLElement>()

  $effect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      if (wrap && !wrap.contains(e.target as Node)) open = false
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') open = false
    }
    document.addEventListener('mousedown', onDown, true)
    document.addEventListener('keydown', onKey, true)
    return () => {
      document.removeEventListener('mousedown', onDown, true)
      document.removeEventListener('keydown', onKey, true)
    }
  })
</script>

<span class="menu-wrap" bind:this={wrap}>
  <button
    class="group-btn"
    class:open
    {disabled}
    aria-expanded={open}
    aria-haspopup="menu"
    onclick={() => (open = !open)}
  >
    {#if icon}<span class="ico">{@render icon()}</span>{/if}
    <span class="lbl">{label}</span>
    <span class="chev" aria-hidden="true">▾</span>
  </button>

  {#if open}
    <!-- Clicking a menu-item button closes the menu; clicking a form control inside it does not, so a
         dropdown or number field can be used without the menu snapping shut. The items themselves are
         real buttons, so this wrapper's click is a convenience only. -->
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div
      class="menu"
      role="menu"
      class:right={align === 'right'}
      onclick={(e) => {
        if ((e.target as HTMLElement).closest('.mi')) open = false
      }}
    >
      {@render children()}
    </div>
  {/if}
</span>

<style>
  .menu-wrap {
    position: relative;
    display: inline-flex;
  }

  .group-btn {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    white-space: nowrap;
  }

  .group-btn.open {
    border-color: var(--accent, var(--line));
  }

  .ico {
    display: inline-flex;
    width: 15px;
    height: 15px;
  }

  .ico :global(svg) {
    width: 15px;
    height: 15px;
  }

  .chev {
    font-size: 10px;
    color: var(--muted);
  }

  .menu {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 60;
    display: grid;
    gap: 2px;
    min-width: 200px;
    padding: 6px;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 10px;
    box-shadow: 0 14px 36px rgba(0, 0, 0, 0.5);
  }

  .menu.right {
    left: auto;
    right: 0;
  }

  /* Uniform item look for whatever the caller renders (buttons/labels). */
  .menu :global(.mi) {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 7px 9px;
    text-align: left;
    background: transparent;
    border: 0;
    border-radius: 7px;
    color: var(--text);
    font-size: 13px;
    cursor: pointer;
  }

  .menu :global(.mi:hover) {
    background: var(--bg);
  }

  .menu :global(.mi:disabled) {
    opacity: 0.5;
    cursor: default;
  }

  .menu :global(.mi.on) {
    color: var(--accent);
    font-weight: 600;
  }

  .menu :global(.mi.danger:hover) {
    color: var(--danger);
  }

  .menu :global(.mi svg) {
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    opacity: 0.85;
  }

  .menu :global(.sep) {
    height: 1px;
    margin: 4px 2px;
    background: var(--line);
  }

  .menu :global(.mhead) {
    padding: 4px 9px 2px;
    color: var(--muted);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
</style>
