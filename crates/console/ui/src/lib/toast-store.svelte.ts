// Small, transient notifications that appear at the bottom-right and fade away on their own — so
// something happening (a PC joining, an error) never takes over the screen.

export type ToastKind = 'ok' | 'error' | 'info'

export interface Toast {
  id: number
  message: string
  kind: ToastKind
}

class ToastStore {
  items = $state<Toast[]>([])
  private next = 1

  push(message: string, kind: ToastKind = 'info', ms = 4000) {
    const id = this.next++
    this.items.push({ id, message, kind })
    if (this.items.length > 5) this.items.shift() // never stack more than a handful
    setTimeout(() => this.dismiss(id), ms)
  }

  dismiss(id: number) {
    this.items = this.items.filter((t) => t.id !== id)
  }
}

export const toasts = new ToastStore()
