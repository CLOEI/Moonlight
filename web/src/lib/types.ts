export type AuthKind = "android_device" | "jwt" | "email_password"

export type AuthInput =
  | {
      kind: "android_device"
      device_id: string | null
    }
  | {
      kind: "jwt"
      jwt: string
      device_id: string | null
    }
  | {
      kind: "email_password"
      email: string
      password: string
      device_id: string | null
    }

export type SessionStatus =
  | "idle"
  | "connecting"
  | "authenticating"
  | "menu_ready"
  | "joining_world"
  | "loading_world"
  | "awaiting_ready"
  | "in_world"
  | "redirecting"
  | "disconnected"
  | "error"

export type PlayerPosition = {
  map_x: number | null
  map_y: number | null
  world_x: number | null
  world_y: number | null
}

export type RemotePlayerSnapshot = {
  user_id: string
  position: PlayerPosition
}

export type InventoryItem = {
  block_id: number
  inventory_type: number
  amount: number
}

export type WorldSnapshot = {
  world_name: string | null
  width: number
  height: number
}

export type SessionSnapshot = {
  id: string
  status: SessionStatus
  device_id: string
  current_host: string
  current_port: number
  current_world: string | null
  pending_world: string | null
  username: string | null
  user_id: string | null
  world: WorldSnapshot | null
  player_position: PlayerPosition
  inventory: InventoryItem[]
  last_error: string | null
}

export type MinimapSnapshot = {
  width: number
  height: number
  foreground_tiles: number[]
  background_tiles: number[]
  water_tiles: number[]
  wiring_tiles: number[]
  player_position: PlayerPosition
  other_players: RemotePlayerSnapshot[]
}

export type ApiMessage = {
  ok: boolean
  message: string
}

export type LuaScriptStatusSnapshot = {
  running: boolean
  started_at: number | null
  finished_at: number | null
  last_error: string | null
  last_result_message: string | null
}

export type LogEvent = {
  timestamp_ms: number
  level: string
  transport: string | null
  direction: string | null
  scope: string
  session_id: string | null
  message: string
  formatted: string
}

export type TutorialCompletedEvent = {
  timestamp_ms: number
  session_id: string
  message: string
}

export type ServerEvent =
  | {
      type: "log"
      event: LogEvent
    }
  | {
      type: "session"
      snapshot: SessionSnapshot
    }
  | {
      type: "tutorial_completed"
      event: TutorialCompletedEvent
    }

export type BlockNameMap = Record<string, string>
