import { memo, useCallback, useEffect, useRef, useState } from "react"

import { minimapColor } from "@/lib/dashboard"
import type { MinimapSnapshot, PlayerPosition } from "@/lib/types"
import { getAtlas, peekAtlas, type AtlasBundle } from "@/lib/atlas"

type Props = {
  minimap: MinimapSnapshot | null
  playerPosition: PlayerPosition
  currentWorld: string | null
  onHoverChange: (value: string) => void
}

const MIN_ZOOM = 0.5
const MAX_ZOOM = 16

function unpackColor(value: number): [number, number, number] {
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255]
}

function blend(
  base: [number, number, number],
  overlay: [number, number, number],
  alpha: number,
): [number, number, number] {
  return [
    Math.round(base[0] * (1 - alpha) + overlay[0] * alpha),
    Math.round(base[1] * (1 - alpha) + overlay[1] * alpha),
    Math.round(base[2] * (1 - alpha) + overlay[2] * alpha),
  ]
}

function rasterize(
  snap: MinimapSnapshot,
  atlasTiles?: Record<string, [number, number]>,
): ImageData {
  const { width, height, foreground_tiles, background_tiles, water_tiles, wiring_tiles } = snap
  const data = new Uint8ClampedArray(width * height * 4)
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const i = y * width + x
      const screenY = height - 1 - y
      const o = (screenY * width + x) * 4
      const bg = background_tiles[i] ?? 0
      const fg = foreground_tiles[i] ?? 0
      const w = water_tiles[i] ?? 0
      const wr = wiring_tiles[i] ?? 0
      let c: [number, number, number] = bg
        ? unpackColor(minimapColor(bg, "background"))
        : [16, 22, 34]
      const fgHasSprite = !!fg && !!atlasTiles && atlasTiles[String(fg)] !== undefined
      if (fg && !fgHasSprite) c = unpackColor(minimapColor(fg, "foreground"))
      if (w) c = blend(c, unpackColor(minimapColor(w, "water")), 0.55)
      if (wr) c = blend(c, unpackColor(minimapColor(wr, "wiring")), 0.45)
      data[o] = c[0]
      data[o + 1] = c[1]
      data[o + 2] = c[2]
      data[o + 3] = 255
    }
  }
  return new ImageData(data, width, height)
}

interface TileInfo {
  mapX: number
  mapY: number
  fg: number
  bg: number
  water: number
  wiring: number
}

function MinimapPanelImpl({ minimap, playerPosition, currentWorld, onHoverChange }: Props) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const tileLayerRef = useRef<HTMLCanvasElement | null>(null)
  const atlasImgRef = useRef<HTMLImageElement | null>(null)
  const atlasMetaRef = useRef<{ cell: number; tiles: Record<string, [number, number]> } | null>(
    null,
  )
  const [atlasResolved, setAtlasResolved] = useState(() => peekAtlas() !== null)
  const [hover, setHover] = useState<TileInfo | null>(null)
  const [view, setView] = useState({ zoom: 1, panX: 0, panY: 0 })
  const viewRef = useRef(view)
  const layoutRef = useRef({ dx: 0, dy: 0, scale: 1, w: 0, h: 0 })
  const onHoverChangeRef = useRef(onHoverChange)
  onHoverChangeRef.current = onHoverChange

  useEffect(() => {
    const cached = peekAtlas()
    if (cached) {
      atlasImgRef.current = cached.image
      atlasMetaRef.current = cached.meta
      setAtlasResolved(true)
      return
    }
    let cancelled = false
    getAtlas().then((bundle: AtlasBundle | null) => {
      if (cancelled) return
      if (bundle) {
        atlasImgRef.current = bundle.image
        atlasMetaRef.current = bundle.meta
      }
      setAtlasResolved(true)
    })
    return () => {
      cancelled = true
    }
  }, [])

  const minimapRef = useRef<MinimapSnapshot | null>(minimap)
  minimapRef.current = minimap

  const lastWorldRef = useRef<string | null>(null)
  useEffect(() => {
    if (lastWorldRef.current !== currentWorld) {
      lastWorldRef.current = currentWorld
      setView({ zoom: 1, panX: 0, panY: 0 })
    }
  }, [currentWorld])

  useEffect(() => {
    viewRef.current = view
  }, [view])

  useEffect(() => {
    if (!minimap || !atlasResolved) {
      tileLayerRef.current = null
      return
    }
    const off = document.createElement("canvas")
    off.width = minimap.width
    off.height = minimap.height
    const ctx = off.getContext("2d")
    if (!ctx) return
    ctx.putImageData(rasterize(minimap, atlasMetaRef.current?.tiles), 0, 0)
    tileLayerRef.current = off
  }, [minimap, atlasResolved])

  const drawScene = useCallback(() => {
    const canvas = canvasRef.current
    const layer = tileLayerRef.current
    if (!canvas || !minimap) return
    const ctx = canvas.getContext("2d")
    if (!ctx) return

    const cw = canvas.clientWidth
    const ch = canvas.clientHeight
    if (canvas.width !== cw || canvas.height !== ch) {
      canvas.width = cw
      canvas.height = ch
    }

    const v = viewRef.current
    const fitScale = Math.min(cw / minimap.width, ch / minimap.height)
    const scale = fitScale * v.zoom
    const dw = minimap.width * scale
    const dh = minimap.height * scale
    const dx = (cw - dw) / 2 + v.panX
    const dy = (ch - dh) / 2 + v.panY
    layoutRef.current = { dx, dy, scale, w: minimap.width, h: minimap.height }

    ctx.imageSmoothingEnabled = false
    ctx.fillStyle = "#0a0f18"
    ctx.fillRect(0, 0, cw, ch)
    if (layer) {
      ctx.drawImage(layer, dx, dy, dw, dh)
    }

    const atlas = atlasImgRef.current
    const meta = atlasMetaRef.current
    if (atlas && meta && scale >= 3) {
      const cell = meta.cell
      const fg = minimap.foreground_tiles
      const bg = minimap.background_tiles
      const xMin = Math.max(0, Math.floor(-dx / scale))
      const xMax = Math.min(minimap.width, Math.ceil((cw - dx) / scale))
      const yScreenMin = Math.max(0, Math.floor(-dy / scale))
      const yScreenMax = Math.min(minimap.height, Math.ceil((ch - dy) / scale))
      const yMin = minimap.height - yScreenMax
      const yMax = minimap.height - yScreenMin

      ctx.globalAlpha = 0.5
      for (let y = yMin; y < yMax; y += 1) {
        for (let x = xMin; x < xMax; x += 1) {
          const bgId = bg[y * minimap.width + x]
          if (!bgId) continue
          const pos = meta.tiles[String(bgId)]
          if (!pos) continue
          const screenY = minimap.height - 1 - y
          ctx.drawImage(
            atlas,
            pos[0],
            pos[1],
            cell,
            cell,
            dx + x * scale,
            dy + screenY * scale,
            scale,
            scale,
          )
        }
      }
      ctx.globalAlpha = 1.0

      for (let y = yMin; y < yMax; y += 1) {
        for (let x = xMin; x < xMax; x += 1) {
          const fgId = fg[y * minimap.width + x]
          if (!fgId) continue
          const pos = meta.tiles[String(fgId)]
          if (!pos) continue
          const screenY = minimap.height - 1 - y
          ctx.drawImage(
            atlas,
            pos[0],
            pos[1],
            cell,
            cell,
            dx + x * scale,
            dy + screenY * scale,
            scale,
            scale,
          )
        }
      }
    }

    if (hover) {
      const hx = dx + hover.mapX * scale
      const hy = dy + (minimap.height - hover.mapY - 1) * scale
      ctx.strokeStyle = "#ffffff"
      ctx.lineWidth = 1.5
      ctx.strokeRect(hx, hy, scale, scale)
    }

    for (const op of minimap.other_players) {
      if (op.position.map_x == null || op.position.map_y == null) continue
      const sx = dx + op.position.map_x * scale + scale / 2
      const sy = dy + (minimap.height - op.position.map_y - 1) * scale + scale / 2
      ctx.fillStyle = "#a855f7"
      ctx.beginPath()
      ctx.arc(sx, sy, Math.max(2, scale * 0.45), 0, Math.PI * 2)
      ctx.fill()
    }

    const px = playerPosition.map_x
    const py = playerPosition.map_y
    if (px != null && py != null) {
      const sx = dx + px * scale + scale / 2
      const sy = dy + (minimap.height - py - 1) * scale + scale / 2
      ctx.fillStyle = "#ff3b30"
      ctx.beginPath()
      ctx.arc(sx, sy, Math.max(3, scale * 0.6), 0, Math.PI * 2)
      ctx.fill()
      ctx.strokeStyle = "#ffffff"
      ctx.lineWidth = 1
      ctx.stroke()
    }
  }, [minimap, hover, playerPosition.map_x, playerPosition.map_y])

  const drawSceneRef = useRef(drawScene)
  drawSceneRef.current = drawScene

  useEffect(() => {
    drawSceneRef.current()
  }, [minimap, atlasResolved, view, hover, playerPosition.map_x, playerPosition.map_y])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const observer = new ResizeObserver(() => drawSceneRef.current())
    observer.observe(canvas)
    return () => observer.disconnect()
  }, [])

  const screenToTile = useCallback(
    (clientX: number, clientY: number): TileInfo | null => {
      if (!minimap) return null
      const canvas = canvasRef.current
      if (!canvas) return null
      const rect = canvas.getBoundingClientRect()
      const cx = clientX - rect.left
      const cy = clientY - rect.top
      const { dx, dy, scale, w, h } = layoutRef.current
      const mapX = Math.floor((cx - dx) / scale)
      const screenY = Math.floor((cy - dy) / scale)
      const mapY = h - 1 - screenY
      if (mapX < 0 || mapX >= w || mapY < 0 || mapY >= h) return null
      const i = mapY * w + mapX
      return {
        mapX,
        mapY,
        fg: minimap.foreground_tiles[i] ?? 0,
        bg: minimap.background_tiles[i] ?? 0,
        water: minimap.water_tiles[i] ?? 0,
        wiring: minimap.wiring_tiles[i] ?? 0,
      }
    },
    [minimap],
  )

  const dragRef = useRef<{
    startX: number
    startY: number
    panX0: number
    panY0: number
    moved: boolean
  } | null>(null)

  const lastHoverKeyRef = useRef<string>("")
  const updateHover = useCallback((tile: TileInfo | null) => {
    const key = tile ? `${tile.mapX},${tile.mapY}` : ""
    if (key === lastHoverKeyRef.current) return
    lastHoverKeyRef.current = key
    setHover(tile)
    if (!tile) {
      onHoverChangeRef.current("Hover a tile to inspect it.")
      return
    }
    onHoverChangeRef.current(
      `hover tile=(${tile.mapX}, ${tile.mapY}) fg=${tile.fg} bg=${tile.bg} water=${tile.water} wiring=${tile.wiring}`,
    )
  }, [])

  const onPointerDown = useCallback((e: React.PointerEvent) => {
    if (e.button !== 0) return
    const v = viewRef.current
    dragRef.current = {
      startX: e.clientX,
      startY: e.clientY,
      panX0: v.panX,
      panY0: v.panY,
      moved: false,
    }
    ;(e.currentTarget as HTMLCanvasElement).setPointerCapture(e.pointerId)
  }, [])

  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      const tile = screenToTile(e.clientX, e.clientY)
      updateHover(tile)
      const drag = dragRef.current
      if (!drag) return
      const dx = e.clientX - drag.startX
      const dy = e.clientY - drag.startY
      if (!drag.moved && Math.abs(dx) + Math.abs(dy) > 5) {
        drag.moved = true
      }
      if (drag.moved) {
        e.preventDefault()
        setView((prev) => ({ ...prev, panX: drag.panX0 + dx, panY: drag.panY0 + dy }))
      }
    },
    [screenToTile, updateHover],
  )

  const onPointerUp = useCallback((e: React.PointerEvent) => {
    if (dragRef.current) {
      try {
        ;(e.currentTarget as HTMLCanvasElement).releasePointerCapture(e.pointerId)
      } catch {
        /* noop */
      }
    }
    dragRef.current = null
  }, [])

  const onPointerLeave = useCallback(() => {
    updateHover(null)
  }, [updateHover])

  useEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const handler = (e: WheelEvent) => {
      if (!minimapRef.current) return
      e.preventDefault()
      const rect = canvas.getBoundingClientRect()
      const cx = e.clientX - rect.left
      const cy = e.clientY - rect.top
      const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15
      const halfW = canvas.clientWidth / 2
      const halfH = canvas.clientHeight / 2
      setView((v) => {
        const nextZoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v.zoom * factor))
        const ratio = v.zoom > 0 ? nextZoom / v.zoom : 1
        return {
          zoom: nextZoom,
          panX: cx - halfW - (cx - halfW - v.panX) * ratio,
          panY: cy - halfH - (cy - halfH - v.panY) * ratio,
        }
      })
    }
    canvas.addEventListener("wheel", handler, { passive: false })
    return () => canvas.removeEventListener("wheel", handler)
  }, [])

  return (
    <div className="relative h-80 overflow-hidden rounded-2xl border border-white/10 bg-[#081018]">
      <canvas
        ref={canvasRef}
        className="absolute inset-0 h-full w-full cursor-crosshair"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        onPointerLeave={onPointerLeave}
        style={{ touchAction: "none" }}
      />
      {!minimap && (
        <div className="absolute inset-0 flex items-center justify-center text-sm text-muted-foreground">
          No minimap yet.
        </div>
      )}
    </div>
  )
}

export const MinimapPanel = memo(MinimapPanelImpl, (prev, next) => {
  return (
    prev.minimap === next.minimap &&
    prev.onHoverChange === next.onHoverChange &&
    prev.currentWorld === next.currentWorld &&
    prev.playerPosition.map_x === next.playerPosition.map_x &&
    prev.playerPosition.map_y === next.playerPosition.map_y &&
    prev.playerPosition.world_x === next.playerPosition.world_x &&
    prev.playerPosition.world_y === next.playerPosition.world_y
  )
})
