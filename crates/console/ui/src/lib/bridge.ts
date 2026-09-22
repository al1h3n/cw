/**
 * One transport for the whole UI, so every component is identical in the desktop app and the browser.
 *
 * In the Tauri window the real `@tauri-apps/api` is used. In `cowatcher-console web` (a plain browser,
 * where `window.__TAURI_INTERNALS__` is absent) `invoke(cmd, args)` becomes `POST /invoke/{cmd}` and
 * `listen(event)` becomes a subscription to the shared `/events` SSE stream. Components import
 * `invoke`/`listen` from here instead of `@tauri-apps/api/*`.
 */
import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen as tauriListen, type UnlistenFn } from '@tauri-apps/api/event'

/** True inside the Tauri desktop shell; false in an ordinary browser tab. */
export const isTauri = typeof (window as any).__TAURI_INTERNALS__ !== 'undefined'

/** Calls a console command. Mirrors Tauri's `invoke`. */
export async function invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) return tauriInvoke<T>(cmd, args)
  const res = await fetch(`/invoke/${cmd}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(args ?? {}),
  })
  const text = await res.text()
  const data = text ? JSON.parse(text) : null
  if (!res.ok) {
    // The server sends `{ error }` on failure so the message reads like a real error.
    throw data && typeof data === 'object' && 'error' in data ? (data as any).error : text || res.statusText
  }
  return data as T
}

// --- SSE event bridge (browser only) ---------------------------------------------------------------
type Handler = (event: { payload: any }) => void
const handlers = new Map<string, Set<Handler>>()
let source: EventSource | null = null

function ensureStream() {
  if (source || isTauri) return
  source = new EventSource('/events')
  source.onmessage = (message) => {
    try {
      const { event, payload } = JSON.parse(message.data)
      handlers.get(event)?.forEach((handler) => handler({ payload }))
    } catch {
      // Ignore a malformed frame; the next full poll re-syncs the UI anyway.
    }
  }
  // The browser reconnects an errored EventSource on its own, so nothing to do here.
}

/** Subscribes to a console event. Mirrors Tauri's `listen`, returning an unlisten function. */
export async function listen<T = unknown>(
  event: string,
  handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  if (isTauri) return tauriListen<T>(event, handler)
  ensureStream()
  let set = handlers.get(event)
  if (!set) {
    set = new Set()
    handlers.set(event, set)
  }
  set.add(handler as Handler)
  return () => {
    set!.delete(handler as Handler)
  }
}

export type { UnlistenFn }
