<script lang="ts">
  // A small, draggable, icon-only launcher for Surey. No text, no gradient, no stray dot — just a
  // clean round button you can drop anywhere. A press that doesn't move opens the panel; a press
  // that moves repositions the button (and remembers where).
  let { onopen }: { onopen: () => void } = $props()

  let pos = $state({ x: window.innerWidth - 76, y: window.innerHeight - 84 })
  let moved = false
  let start = { x: 0, y: 0, px: 0, py: 0 }

  try {
    const raw = localStorage.getItem('surey.fab')
    if (raw) pos = JSON.parse(raw)
  } catch {
    /* first run / private window */
  }
  // Keep it on-screen if the window shrank since last time.
  pos = {
    x: Math.min(pos.x, window.innerWidth - 60),
    y: Math.min(pos.y, window.innerHeight - 60),
  }

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

<button
  class="launcher"
  style="left:{pos.x}px; top:{pos.y}px;"
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  aria-label="Surey"
  title="Surey"
>
  <!-- A four-point spark: distinct from the app's other glyphs, reads as "assistant". -->
  <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true">
    <path
      d="M12 2.5c.5 3.6 1.9 5 5.5 5.5-3.6.5-5 1.9-5.5 5.5-.5-3.6-1.9-5-5.5-5.5 3.6-.5 5-1.9 5.5-5.5Z"
      fill="currentColor"
    />
    <path
      d="M18.5 13.5c.28 1.8 1 2.5 2.8 2.8-1.8.28-2.5 1-2.8 2.8-.28-1.8-1-2.5-2.8-2.8 1.8-.28 2.5-1 2.8-2.8Z"
      fill="currentColor"
      opacity="0.75"
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
  .launcher svg {
    display: block;
    pointer-events: none;
  }
</style>
