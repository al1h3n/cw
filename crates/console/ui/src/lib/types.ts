export type DeviceStatus = 'idle' | 'connecting' | 'live' | 'offline'

export interface Device {
  device_id: string
  key: string
  status: DeviceStatus
  /** Latest screen as a data: URL, or null before the first frame arrives. */
  screen: string | null
  detail: string | null
}

export interface ConsoleInfo {
  device_id: string
  public_key: string
}

export interface PairingInvite {
  code: string
  command: string
}
