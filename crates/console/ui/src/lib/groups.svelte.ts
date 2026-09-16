// The teacher's arrangement of student tiles into named groups, persisted locally.
//
// This is a console-side view concern (like the custom names), so it lives in the webview's
// localStorage rather than the wire protocol. Every paired PC belongs to exactly one group;
// newly paired ones land in "Uncategorised", which always exists and cannot be deleted.

export interface Group {
  id: string
  name: string
  collapsed: boolean
}

interface Layout {
  groups: Group[]
  members: Record<string, string[]>
}

const KEY = 'cowatcher.layout.v1'
export const UNCAT = 'uncategorised'

function fresh(): Layout {
  return {
    groups: [{ id: UNCAT, name: 'Uncategorised', collapsed: false }],
    members: { [UNCAT]: [] },
  }
}

function load(): Layout {
  try {
    const raw = localStorage.getItem(KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as Layout
      if (Array.isArray(parsed.groups) && parsed.members && typeof parsed.members === 'object') {
        return parsed
      }
    }
  } catch {
    // corrupt or unavailable storage: start clean rather than break the grid
  }
  return fresh()
}

class GroupStore {
  layout = $state<Layout>(load())

  private persist() {
    try {
      localStorage.setItem(KEY, JSON.stringify(this.layout))
    } catch {
      // private window / storage disabled: the arrangement just will not survive a restart
    }
  }

  /** Persist the current arrangement (call after a drag settles). */
  save() {
    this.persist()
  }

  /** Reconcile the arrangement with the real device list: keep every known PC exactly once,
   *  drop any that are no longer paired, and drop newcomers into Uncategorised. */
  sync(deviceIds: string[]) {
    const l = this.layout
    if (!l.groups.some((g) => g.id === UNCAT)) {
      l.groups.push({ id: UNCAT, name: 'Uncategorised', collapsed: false })
    }
    if (!l.members[UNCAT]) l.members[UNCAT] = []

    const known = new Set(deviceIds)
    const placed = new Set<string>()
    for (const g of l.groups) {
      const kept = (l.members[g.id] ?? []).filter((id) => {
        if (!known.has(id) || placed.has(id)) return false
        placed.add(id)
        return true
      })
      l.members[g.id] = kept
    }
    let changed = false
    for (const id of deviceIds) {
      if (!placed.has(id)) {
        l.members[UNCAT].push(id)
        changed = true
      }
    }
    if (changed) this.persist()
  }

  get groups(): Group[] {
    return this.layout.groups
  }

  members(id: string): string[] {
    return this.layout.members[id] ?? []
  }

  isUncat(id: string): boolean {
    return id === UNCAT
  }

  addGroup(name = 'New group'): string {
    const id = 'g' + Math.random().toString(36).slice(2, 9)
    // Insert before Uncategorised, which stays last.
    const uncatIdx = this.layout.groups.findIndex((g) => g.id === UNCAT)
    const at = uncatIdx < 0 ? this.layout.groups.length : uncatIdx
    this.layout.groups.splice(at, 0, { id, name: name.trim() || 'New group', collapsed: false })
    this.layout.members[id] = []
    this.persist()
    return id
  }

  rename(id: string, name: string) {
    if (id === UNCAT) return
    const g = this.layout.groups.find((g) => g.id === id)
    if (g) {
      g.name = name.trim() || g.name
      this.persist()
    }
  }

  toggle(id: string) {
    const g = this.layout.groups.find((g) => g.id === id)
    if (g) {
      g.collapsed = !g.collapsed
      this.persist()
    }
  }

  remove(id: string) {
    if (id === UNCAT) return
    const members = this.layout.members[id] ?? []
    if (!this.layout.members[UNCAT]) this.layout.members[UNCAT] = []
    this.layout.members[UNCAT].push(...members)
    delete this.layout.members[id]
    this.layout.groups = this.layout.groups.filter((g) => g.id !== id)
    this.persist()
  }

  /** Move a device to `toGroup` at `toIndex`. Does NOT persist — call save() when the drag settles,
   *  so a live drag can reorder freely (animated) without hammering storage. */
  move(deviceId: string, toGroup: string, toIndex: number) {
    for (const g of this.layout.groups) {
      const arr = this.layout.members[g.id]
      if (!arr) continue
      const i = arr.indexOf(deviceId)
      if (i >= 0) arr.splice(i, 1)
    }
    if (!this.layout.members[toGroup]) this.layout.members[toGroup] = []
    const target = this.layout.members[toGroup]
    const idx = Math.max(0, Math.min(toIndex, target.length))
    target.splice(idx, 0, deviceId)
  }

  /** Where a device currently is, for menu highlighting. */
  groupOf(deviceId: string): string {
    for (const g of this.layout.groups) {
      if ((this.layout.members[g.id] ?? []).includes(deviceId)) return g.id
    }
    return UNCAT
  }
}

export const groups = new GroupStore()
