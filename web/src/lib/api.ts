import { buildBackendUrl } from "@/lib/dashboard"
import type {
  ApiMessage,
  AuthInput,
  BlockNameMap,
  DashboardAuthResponse,
  DashboardAuthStatus,
  LuaScriptStatusSnapshot,
  MinimapSnapshot,
  SessionSnapshot,
} from "@/lib/types"

export type ActionResponse = {
  result?: ApiMessage
  session?: SessionSnapshot
  ok?: boolean
  message?: string
}

type SessionsResponse = {
  sessions?: SessionSnapshot[]
}

type MinimapResponse = {
  minimap?: MinimapSnapshot | null
}

type LuaStatusResponse = {
  status?: LuaScriptStatusSnapshot | null
}

const AUTH_TOKEN_KEY = "moonlight_dashboard_token"

export function getAuthToken() {
  if (typeof window === "undefined") {
    return null
  }
  return window.localStorage.getItem(AUTH_TOKEN_KEY)
}

export function setAuthToken(token: string | null) {
  if (typeof window === "undefined") {
    return
  }
  if (token) {
    window.localStorage.setItem(AUTH_TOKEN_KEY, token)
  } else {
    window.localStorage.removeItem(AUTH_TOKEN_KEY)
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const authToken = getAuthToken()
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    ...(init?.headers ? (init.headers as Record<string, string>) : {}),
  }
  if (authToken) {
    headers.Authorization = `Bearer ${authToken}`
  }
  const response = await fetch(buildBackendUrl(path), {
    headers,
    ...init,
  })

  const text = await response.text()
  const contentType = response.headers.get("content-type") ?? ""
  if (text && !contentType.includes("application/json")) {
    throw new Error(`expected JSON from ${path}, got ${contentType || "non-JSON response"}`)
  }
  const payload = text ? (JSON.parse(text) as T) : ({} as T)
  const message = text ? (payload as { message?: string }).message : undefined
  if (!response.ok) {
    throw new Error(message ?? `request failed: ${response.status}`)
  }
  return payload
}

export function getDashboardAuthStatus() {
  return request<DashboardAuthStatus>("/api/auth/status")
}

export function registerDashboardPassword(password: string) {
  return request<DashboardAuthResponse>("/api/auth/register", {
    method: "POST",
    body: JSON.stringify({ password }),
  })
}

export function loginDashboard(password: string) {
  return request<DashboardAuthResponse>("/api/auth/login", {
    method: "POST",
    body: JSON.stringify({ password }),
  })
}

export function logoutDashboard() {
  return request<DashboardAuthResponse>("/api/auth/logout", {
    method: "POST",
  })
}

export function connectWithAuth(auth: AuthInput) {
  return request<ActionResponse>("/api/connect", {
    method: "POST",
    body: JSON.stringify({ auth }),
  })
}

export function listSessions() {
  return request<SessionsResponse>("/api/sessions")
}

export function joinWorld(sessionId: string, world: string, instance = false) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/join`, {
    method: "POST",
    body: JSON.stringify({ world, instance }),
  })
}

export function leaveWorld(sessionId: string) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/leave`, {
    method: "POST",
  })
}

export function disconnectSession(sessionId: string) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/disconnect`, {
    method: "POST",
  })
}

export function reconnectSession(sessionId: string) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/reconnect`, {
    method: "POST",
  })
}

export function automateTutorial(sessionId: string) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/tutorial/automate`, {
    method: "POST",
  })
}

export function moveSession(sessionId: string, direction: string) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/move`, {
    method: "POST",
    body: JSON.stringify({ direction }),
  })
}

export function wearItem(sessionId: string, blockId: number, equip: boolean) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/wear`, {
    method: "POST",
    body: JSON.stringify({ block_id: blockId, equip }),
  })
}

export function getMinimap(sessionId: string) {
  return request<MinimapResponse>(`/api/sessions/${sessionId}/minimap?ts=${Date.now()}`)
}

export function startFishing(sessionId: string, direction: string, bait: string) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/fishing/start`, {
    method: "POST",
    body: JSON.stringify({ direction, bait }),
  })
}

export function stopFishing(sessionId: string) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/fishing/stop`, {
    method: "POST",
  })
}

export function talk(sessionId: string, message: string) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/talk`, {
    method: "POST",
    body: JSON.stringify({ message }),
  })
}

export function startSpam(sessionId: string, message: string, delayMs: number) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/spam/start`, {
    method: "POST",
    body: JSON.stringify({ message, delay_ms: delayMs }),
  })
}

export function stopSpam(sessionId: string) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/spam/stop`, {
    method: "POST",
  })
}

export function loadBlockTypes() {
  return request<BlockNameMap>("/block_types.json")
}

export function startLuaScript(sessionId: string, source: string) {
  return request<ActionResponse & LuaStatusResponse>(`/api/sessions/${sessionId}/lua/start`, {
    method: "POST",
    body: JSON.stringify({ source }),
  })
}

export function stopLuaScript(sessionId: string) {
  return request<ActionResponse & LuaStatusResponse>(`/api/sessions/${sessionId}/lua/stop`, {
    method: "POST",
  })
}

export function getLuaScriptStatus(sessionId: string) {
  return request<LuaStatusResponse>(`/api/sessions/${sessionId}/lua/status?ts=${Date.now()}`)
}
