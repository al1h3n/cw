export type DeviceStatus = 'idle' | 'connecting' | 'live' | 'offline'

export interface Monitor {
  index: number
  width: number
  height: number
  primary: boolean
}

export interface Device {
  device_id: string
  key: string
  status: DeviceStatus
  /** Latest screen as a data: URL, or null before the first frame arrives. */
  screen: string | null
  detail: string | null
  /** Monitors this PC reported; empty until it connects. */
  monitors: Monitor[]
  /** Which monitor is currently shown. */
  monitor: number
  /** The answer to the last lock/power action sent to this PC. */
  last_action: ActionReport | null
}

export interface ActionReport {
  action: string
  /** 'started' or a failure code; translated with the `result_*` strings. */
  result: string
  delay_seconds: number
  at_ms: number
}

export interface PreviewWidths {
  grid: number
  focused: number
}

export interface ConsoleInfo {
  device_id: string
  public_key: string
}

export interface PairingInvite {
  code: string
  command: string
}

export interface RecordingInfo {
  active: boolean
  file: string
  frames: number
  width: number
  height: number
  fps: number
  problem: string
}

export interface AppEntry {
  id: number
  name: string
}

export interface RunningApp {
  pid: number
  name: string
}
