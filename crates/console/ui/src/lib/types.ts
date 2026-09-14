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
