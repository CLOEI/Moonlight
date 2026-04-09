import type {
  MinimapSnapshot,
  SessionSnapshot,
  SessionStatus,
} from "@/lib/types"

export function generateDeviceId() {
  const bytes = new Uint8Array(20)
  crypto.getRandomValues(bytes)
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")
}

export function stripAnsi(value: string) {
  return value.replace(/\u001b\[[0-9;]*m/g, "")
}

export function buildWebSocketUrl() {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:"
  if (import.meta.env.DEV) {
    return `${protocol}//${window.location.hostname}:3000/ws`
  }
  return `${protocol}//${window.location.host}/ws`
}

export function buildBackendUrl(path: string) {
  if (import.meta.env.DEV) {
    return `http://${window.location.hostname}:3000${path}`
  }
  return path
}

export function camelToWords(name: string) {
  return name.replace(/([A-Z])/g, " $1").trim()
}

export function inventoryTypeLabel(inventoryType: number) {
  const labels: Record<number, string> = {
    0: "",
    512: "seed",
    768: "clothing",
    1024: "clothing",
    1280: "background",
    1792: "fishing",
  }
  return labels[inventoryType] ?? `type ${inventoryType}`
}

export function getItemCategory(name: string | undefined, fallback: string) {
  if (name) {
    if (name.startsWith("Weapon")) {
      return "weapon"
    }
    if (name.startsWith("Consumable")) {
      return "consumable"
    }
  }
  return fallback
}

export function sortSessions(sessions: SessionSnapshot[]) {
  return [...sessions].sort((left, right) => left.id.localeCompare(right.id))
}

export function statusVariant(status: SessionStatus) {
  if (status === "error") {
    return "destructive" as const
  }
  if (status === "in_world" || status === "menu_ready") {
    return "secondary" as const
  }
  return "outline" as const
}

export function minimapColor(
  blockId: number,
  layer: "foreground" | "background" | "water" | "wiring",
) {
  if (!blockId) {
    return 0x87ceeb
  }

  let r = (blockId * 85) % 256
  let g = (blockId * 153) % 256
  let b = (blockId * 211) % 256

  if (layer === "background") {
    r = Math.round(r * 0.58)
    g = Math.round(g * 0.58)
    b = Math.round(b * 0.58)
  } else if (layer === "water") {
    r = Math.round((r + 40) * 0.35)
    g = Math.round((g + 160) * 0.65)
    b = Math.round((b + 255) * 0.8)
  } else if (layer === "wiring") {
    r = Math.min(255, Math.round((r + 255) * 0.7))
    g = Math.round((g + 90) * 0.45)
    b = Math.min(255, Math.round((b + 200) * 0.6))
  }

  return (r << 16) | (g << 8) | b
}

export function blendColor(base: number, overlay: number, alpha: number) {
  const baseR = (base >> 16) & 255
  const baseG = (base >> 8) & 255
  const baseB = base & 255
  const overR = (overlay >> 16) & 255
  const overG = (overlay >> 8) & 255
  const overB = overlay & 255

  const r = Math.round(baseR * (1 - alpha) + overR * alpha)
  const g = Math.round(baseG * (1 - alpha) + overG * alpha)
  const b = Math.round(baseB * (1 - alpha) + overB * alpha)

  return (r << 16) | (g << 8) | b
}

export type MinimapView = {
  zoom: number
  minZoom: number
  maxZoom: number
  offsetX: number
  offsetY: number
  hasInteracted: boolean
}

export type MinimapHoverTile = {
  mapX: number
  mapY: number
  foreground: number
  background: number
  water: number
  wiring: number
}

export function fitMinimapView(
  width: number,
  height: number,
  minimapWidth: number,
  minimapHeight: number,
): MinimapView {
  const scaleX = width / minimapWidth
  const scaleY = height / minimapHeight
  const zoom = Math.min(scaleX, scaleY)
  return {
    zoom,
    minZoom: Math.max(zoom * 0.5, 0.25),
    maxZoom: 32,
    offsetX: (width - minimapWidth * zoom) / 2,
    offsetY: (height - minimapHeight * zoom) / 2,
    hasInteracted: false,
  }
}

export function tileAtCanvasPoint(
  point: { x: number; y: number },
  minimap: MinimapSnapshot,
  view: MinimapView,
): MinimapHoverTile | null {
  const tileX = Math.floor((point.x - view.offsetX) / view.zoom)
  const tileYFromTop = Math.floor((point.y - view.offsetY) / view.zoom)
  if (
    tileX < 0 ||
    tileYFromTop < 0 ||
    tileX >= minimap.width ||
    tileYFromTop >= minimap.height
  ) {
    return null
  }

  const mapY = minimap.height - 1 - tileYFromTop
  const index = mapY * minimap.width + tileX

  return {
    mapX: tileX,
    mapY,
    foreground: minimap.foreground_tiles[index] ?? 0,
    background: minimap.background_tiles[index] ?? 0,
    water: minimap.water_tiles[index] ?? 0,
    wiring: minimap.wiring_tiles[index] ?? 0,
  }
}
