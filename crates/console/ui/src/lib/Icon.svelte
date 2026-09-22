<script lang="ts">
  // A tiny set of stroke icons used across the toolbar groups, so buttons are icon + label instead of
  // a wall of text. One component keeps them consistent (same stroke width, same 24-viewport).
  let { name, size = 16 }: { name: string; size?: number } = $props()
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.7"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if name === 'eye'}
    <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7Z" /><circle cx="12" cy="12" r="3" />
  {:else if name === 'eye-off'}
    <path d="M9.9 5.2A9.5 9.5 0 0 1 12 5c6.5 0 10 7 10 7a17 17 0 0 1-3 3.6M6.2 6.2A17 17 0 0 0 2 12s3.5 7 10 7a9.5 9.5 0 0 0 4.1-.9" /><path d="m3 3 18 18" />
  {:else if name === 'plus'}
    <path d="M12 5v14M5 12h14" />
  {:else if name === 'cast'}
    <path d="M2 8V6a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-6" /><path d="M2 12a6 6 0 0 1 6 6M2 16a2 2 0 0 1 2 2" /><path d="M2 20h.01" />
  {:else if name === 'film'}
    <rect x="3" y="4" width="18" height="16" rx="2" /><path d="M7 4v16M17 4v16M3 9h4M3 15h4M17 9h4M17 15h4" />
  {:else if name === 'image'}
    <rect x="3" y="4" width="18" height="16" rx="2" /><circle cx="9" cy="9" r="1.6" /><path d="m4 17 5-4 4 3 3-2 4 3" />
  {:else if name === 'ban'}
    <circle cx="12" cy="12" r="9" /><path d="m5.6 5.6 12.8 12.8" />
  {:else if name === 'lock'}
    <rect x="4.5" y="10.5" width="15" height="10" rx="2" /><path d="M8 10.5V7a4 4 0 0 1 8 0v3.5" />
  {:else if name === 'freeze'}
    <path d="M12 2v20M4 6l16 12M20 6 4 18M2 12h20" />
  {:else if name === 'shield'}
    <path d="M12 3 5 6v5c0 4.5 3 7.5 7 9 4-1.5 7-4.5 7-9V6l-7-3Z" />
  {:else if name === 'power'}
    <path d="M12 3v9" /><path d="M6.4 6.4a8 8 0 1 0 11.2 0" />
  {:else if name === 'refresh'}
    <path d="M21 12a9 9 0 1 1-2.6-6.4M21 4v4h-4" />
  {:else if name === 'settings'}
    <circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-2.7 1.1V21a2 2 0 1 1-4 0v-.1A1.6 1.6 0 0 0 6.8 19l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.6 1.6 0 0 0-1-2.7H3a2 2 0 1 1 0-4h.1A1.6 1.6 0 0 0 4.2 6.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.6 1.6 0 0 0 1.8.3H9a1.6 1.6 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.6 1.6 0 0 0 2.7 1.1l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8V9a1.6 1.6 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.6 1.6 0 0 0-1.5 1Z" />
  {:else if name === 'info'}
    <circle cx="12" cy="12" r="9" /><path d="M12 11v5M12 8h.01" />
  {:else if name === 'monitor'}
    <rect x="2" y="4" width="20" height="13" rx="2" /><path d="M8 21h8M12 17v4" />
  {:else if name === 'clock'}
    <circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" />
  {/if}
</svg>
