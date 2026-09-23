<script lang="ts">
  // A small, draggable, icon-only launcher for Surey. No text, no gradient, no stray dot — just a
  // clean round button you can drop anywhere. A press that doesn't move opens the panel; a press
  // that moves repositions the button (and remembers where).
  // `hidden` while the panel is open (the launcher gets out of the way); `busy` pulses a ring so the
  // teacher can see Surey is still working in the background even with the panel closed.
  let {
    onopen,
    hidden = false,
    busy = false,
  }: { onopen: () => void; hidden?: boolean; busy?: boolean } = $props()

  let pos = $state({ x: window.innerWidth - 76, y: window.innerHeight - 84 })
  let moved = false
  let start = { x: 0, y: 0, px: 0, py: 0 }

  try {
    const raw = localStorage.getItem('surey.fab')
    if (raw) pos = JSON.parse(raw)
  } catch {
    /* first run / private window */
  }

  // Keep the button fully on-screen. Runs at start and on every window resize, so restoring a
  // maximized window (or any shrink) never strands the launcher off-screen where it "disappears".
  function clampToView() {
    pos = {
      x: Math.max(8, Math.min(pos.x, window.innerWidth - 56)),
      y: Math.max(8, Math.min(pos.y, window.innerHeight - 56)),
    }
  }
  clampToView()

  function down(e: PointerEvent) {
    moved = false
    start = { x: e.clientX, y: e.clientY, px: pos.x, py: pos.y }
    ;(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId)
  }
  function move(e: PointerEvent) {
    if (!e.currentTarget || (e.buttons & 1) === 0) return
    const dx = e.clientX - start.x
    const dy = e.clientY - start.y
    if (Math.abs(dx) > 4 || Math.abs(dy) > 4) moved = true
    if (moved) {
      pos = {
        x: Math.max(8, Math.min(window.innerWidth - 56, start.px + dx)),
        y: Math.max(8, Math.min(window.innerHeight - 56, start.py + dy)),
      }
    }
  }
  function up() {
    if (moved) {
      try {
        localStorage.setItem('surey.fab', JSON.stringify(pos))
      } catch {
        /* ignore */
      }
    } else {
      onopen()
    }
  }
</script>

<svelte:window on:resize={clampToView} />

<button
  class="launcher"
  class:hidden
  class:busy
  style="left:{pos.x}px; top:{pos.y}px;"
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  aria-label="Surey"
  title="Surey"
>
  <!-- A single four-point spark, symmetric about the centre of the 24×24 box (top 12,4 · right 20,12 ·
       bottom 12,20 · left 4,12), so it sits in the exact optical centre of the round button. -->
  <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
    <path
      d="M12 4C12.6 8.9 15.1 11.4 20 12 15.1 12.6 12.6 15.1 12 20 11.4 15.1 8.9 12.6 4 12 8.9 11.4 11.4 8.9 12 4Z"
      fill="currentColor"
    />
  </svg>
</button>

<style>
  .launcher {
    position: fixed;
    z-index: 50;
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    padding: 0;
    color: #fff;
    background: var(--accent, #3b6fd4);
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: 50%;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.4);
    cursor: grab;
    touch-action: none;
  }
  .launcher:active {
    cursor: grabbing;
  }
  .launcher:hover {
    filter: brightness(1.08);
  }
  .launcher.hidden {
    display: none;
  }
  /* A soft pulsing ring while Surey is still working with the panel closed. */
  .launcher.busy {
    animation: surey-busy 1.4s ease-out infinite;
  }
  @keyframes surey-busy {
    0% {
      box-shadow: 0 6px 18px rgba(0, 0, 0, 0.4), 0 0 0 0 var(--accent, #3b6fd4);
    }
    70% {
      box-shadow: 0 6px 18px rgba(0, 0, 0, 0.4), 0 0 0 9px transparent;
    }
    100% {
      box-shadow: 0 6px 18px rgba(0, 0, 0, 0.4), 0 0 0 0 transparent;
    }
  }
  .launcher svg {
    display: block;
    pointer-events: none;
  }
</style>
