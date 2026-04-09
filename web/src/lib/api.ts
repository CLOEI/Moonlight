import { buildBackendUrl } from "@/lib/dashboard"
import type {
  ApiMessage,
  AuthInput,
  BlockNameMap,
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

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(buildBackendUrl(path), {
    headers: {
      "Content-Type": "application/json",
    },
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

export function connectWithAuth(auth: AuthInput) {
  return request<ActionResponse>("/api/connect", {
    method: "POST",
    body: JSON.stringify({ auth }),
  })
}

export function listSessions() {
  return request<SessionsResponse>("/api/sessions")
}

export function joinWorld(sessionId: string, world: string) {
  return request<ActionResponse>(`/api/sessions/${sessionId}/join`, {
    method: "POST",
    body: JSON.stringify({ world }),
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
