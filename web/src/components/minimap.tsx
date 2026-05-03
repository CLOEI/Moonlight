import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { Application, extend } from "@pixi/react"
import { Container, Graphics, Sprite, Texture } from "pixi.js"

import {
  fitMinimapView,
  minimapColor,
  tileAtCanvasPoint,
  type MinimapHoverTile,
  type MinimapView,
} from "@/lib/dashboard"
import type { MinimapSnapshot, PlayerPosition } from "@/lib/types"
import { getAtlas, peekAtlas, type AtlasBundle } from "@/lib/atlas"

extend({ Container, Graphics, Sprite })

type Props = {
  minimap: MinimapSnapshot | null
  playerPosition: PlayerPosition
  onHoverChange: (value: string) => void
}

const INITIAL_VIEW: MinimapView = {
  zoom: 1,
  minZoom: 1,
  maxZoom: 32,
  offsetX: 0,
  offsetY: 0,
  hasInteracted: false,
}

function unpackColor(value: number): [number, number, number] {
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255]
}

function blendRgb(
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

function rasterizeMinimap(
  snap: MinimapSnapshot,
  atlas: AtlasBundle | null,
): HTMLCanvasElement {
  const { width, height, foreground_tiles, background_tiles, water_tiles, wiring_tiles } = snap
  const tiles = atlas?.meta.tiles

  const colorCanvas = document.createElement("canvas")
  colorCanvas.width = width
  colorCanvas.height = height
  const colorCtx = colorCanvas.getContext("2d")
  if (!colorCtx) return colorCanvas

  const baseImage = colorCtx.createImageData(width, height)
  const data = baseImage.data
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
        : [135, 206, 235]
      const fgHasSprite = !!fg && !!tiles && tiles[String(fg)] !== undefined
      if (fg && !fgHasSprite) c = unpackColor(minimapColor(fg, "foreground"))
      if (w) c = blendRgb(c, unpackColor(minimapColor(w, "water")), 0.55)
      if (wr) c = blendRgb(c, unpackColor(minimapColor(wr, "wiring")), 0.45)

      data[o] = c[0]
      data[o + 1] = c[1]
      data[o + 2] = c[2]
      data[o + 3] = 255
    }
  }
  colorCtx.putImageData(baseImage, 0, 0)

  if (!atlas || !tiles) return colorCanvas

  const cell = atlas.meta.cell
  const fullCanvas = document.createElement("canvas")
  fullCanvas.width = width * cell
  fullCanvas.height = height * cell
  const ctx = fullCanvas.getContext("2d")
  if (!ctx) return colorCanvas

  ctx.imageSmoothingEnabled = false
  ctx.drawImage(colorCanvas, 0, 0, width, height, 0, 0, width * cell, height * cell)

  ctx.globalAlpha = 0.5
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const bgId = background_tiles[y * width + x]
      if (!bgId) continue
      const pos = tiles[String(bgId)]
      if (!pos) continue
      const screenY = height - 1 - y
      ctx.drawImage(atlas.image, pos[0], pos[1], cell, cell, x * cell, screenY * cell, cell, cell)
    }
  }
  ctx.globalAlpha = 1.0

  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const fgId = foreground_tiles[y * width + x]
      if (!fgId) continue
      const pos = tiles[String(fgId)]
      if (!pos) continue
      const screenY = height - 1 - y
      ctx.drawImage(atlas.image, pos[0], pos[1], cell, cell, x * cell, screenY * cell, cell, cell)
    }
  }

  return fullCanvas
}

export function MinimapPanel({ minimap, playerPosition, onHoverChange }: Props) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const [size, setSize] = useState({ width: 0, height: 0 })
  const [view, setView] = useState<MinimapView>(INITIAL_VIEW)
  const [atlas, setAtlas] = useState<AtlasBundle | null>(() => peekAtlas())
  const [texture, setTexture] = useState<Texture>(Texture.EMPTY)

  useEffect(() => {
    if (atlas) return
    let live = true
    getAtlas().then((b) => {
      if (live && b) setAtlas(b)
    })
    return () => {
      live = false
    }
  }, [atlas])

  useEffect(() => {
    if (!minimap) {
      setTexture(Texture.EMPTY)
      return
    }
    const canvas = rasterizeMinimap(minimap, atlas)
    const tex = Texture.from(canvas)
    tex.source.scaleMode = "nearest"
    setTexture(tex)
    return () => {
      tex.destroy(true)
    }
  }, [minimap, atlas])

  useEffect(() => {
    const container = containerRef.current
    if (!container) {
      return
    }
    const observer = new ResizeObserver(([entry]) => {
      setSize({
        width: entry.contentRect.width,
        height: entry.contentRect.height,
      })
    })
    observer.observe(container)
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    if (!minimap || !size.width || !size.height) {
      setView(INITIAL_VIEW)
      return
    }
    setView((current) =>
      current.hasInteracted
        ? current
        : fitMinimapView(size.width, size.height, minimap.width, minimap.height),
    )
  }, [minimap, size.height, size.width])

  const resolution = useMemo(
    () => (typeof window === "undefined" ? 1 : window.devicePixelRatio || 1),
    [],
  )

  const drawPlayer = useCallback(
    (graphics: Graphics) => {
      graphics.clear()
      if (!minimap) {
        return
      }
      for (const otherPlayer of minimap.other_players) {
        const { map_x, map_y } = otherPlayer.position
        if (map_x == null || map_y == null) {
          continue
        }
        const x = Math.min(Math.max(map_x, 0), minimap.width - 1)
        const y = Math.min(Math.max(map_y, 0), minimap.height - 1)
        graphics.setFillStyle({ color: 0xa855f7 })
        graphics.circle(x + 0.5, minimap.height - y - 0.5, 0.32)
        graphics.fill()
      }
      if (playerPosition.map_x == null || playerPosition.map_y == null) {
        return
      }
      const x = Math.min(Math.max(playerPosition.map_x, 0), minimap.width - 1)
      const y = Math.min(Math.max(playerPosition.map_y, 0), minimap.height - 1)
      graphics.setFillStyle({ color: 0xff3b30 })
      graphics.circle(x + 0.5, minimap.height - y - 0.5, 0.4)
      graphics.fill()
      graphics.setStrokeStyle({ color: 0xffffff, width: 0.08 })
      graphics.stroke()
    },
    [minimap, playerPosition.map_x, playerPosition.map_y],
  )

  const drawHover = useCallback(
    (graphics: Graphics) => {
      graphics.clear()
      if (!minimap) {
        return
      }
      const text = containerRef.current?.dataset.hoverTile
      if (!text) {
        return
      }
      const [mapX, mapY] = text.split(",").map(Number)
      if (!Number.isFinite(mapX) || !Number.isFinite(mapY)) {
        return
      }
      graphics.setStrokeStyle({ color: 0xffffff, width: 0.08 })
      graphics.rect(mapX, minimap.height - mapY - 1, 1, 1)
      graphics.stroke()
    },
    [minimap],
  )

  const updateHover = (tile: MinimapHoverTile | null) => {
    if (!containerRef.current) {
      return
    }
    if (!tile) {
      delete containerRef.current.dataset.hoverTile
      onHoverChange("Hover a tile to inspect it.")
      return
    }
    containerRef.current.dataset.hoverTile = `${tile.mapX},${tile.mapY}`
    onHoverChange(
      `hover tile=(${tile.mapX}, ${tile.mapY}) fg=${tile.foreground} bg=${tile.background} water=${tile.water} wiring=${tile.wiring}`,
    )
  }

  const pointerOrigin = useRef<{ x: number; y: number; offsetX: number; offsetY: number } | null>(
    null,
  )

  const handlePointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!minimap || !containerRef.current) {
      return
    }
    if (pointerOrigin.current) {
      event.preventDefault()
    }
    const rect = containerRef.current.getBoundingClientRect()
    const tile = tileAtCanvasPoint(
      {
        x: event.clientX - rect.left,
        y: event.clientY - rect.top,
      },
      minimap,
      view,
    )
    updateHover(tile)

    if (!pointerOrigin.current) {
      return
    }
    setView((current) => ({
      ...current,
      hasInteracted: true,
      offsetX: pointerOrigin.current!.offsetX + (event.clientX - pointerOrigin.current!.x),
      offsetY: pointerOrigin.current!.offsetY + (event.clientY - pointerOrigin.current!.y),
    }))
  }

  const handleWheelEvent = useCallback((event: WheelEvent) => {
    if (!minimap || !containerRef.current) {
      return
    }
    event.preventDefault()
    event.stopPropagation()
    const rect = containerRef.current.getBoundingClientRect()
    const pointX = event.clientX - rect.left
    const pointY = event.clientY - rect.top
    const zoomFactor = event.deltaY < 0 ? 1.15 : 1 / 1.15

    setView((current) => {
      const nextZoom = Math.min(
        current.maxZoom,
        Math.max(current.minZoom, current.zoom * zoomFactor),
      )
      const worldX = (pointX - current.offsetX) / current.zoom
      const worldY = (pointY - current.offsetY) / current.zoom
      return {
        ...current,
        hasInteracted: true,
        zoom: nextZoom,
        offsetX: pointX - worldX * nextZoom,
        offsetY: pointY - worldY * nextZoom,
      }
    })
  }, [minimap])

  useEffect(() => {
    const container = containerRef.current
    if (!container) {
      return
    }
    container.addEventListener("wheel", handleWheelEvent, { passive: false })
    return () => container.removeEventListener("wheel", handleWheelEvent)
  }, [handleWheelEvent])

  return (
    <div
      ref={containerRef}
      className="relative h-80 overflow-hidden rounded-2xl border border-white/10 bg-[#081018]"
      onPointerDown={(event) => {
        event.preventDefault()
        pointerOrigin.current = {
          x: event.clientX,
          y: event.clientY,
          offsetX: view.offsetX,
          offsetY: view.offsetY,
        }
        event.currentTarget.setPointerCapture(event.pointerId)
      }}
      onPointerUp={() => {
        pointerOrigin.current = null
      }}
      onPointerLeave={() => {
        pointerOrigin.current = null
        updateHover(null)
      }}
      onPointerCancel={() => {
        pointerOrigin.current = null
      }}
      onPointerMove={handlePointerMove}
      style={{ touchAction: "none" }}
    >
      {minimap && size.width > 0 && size.height > 0 ? (
        <Application
          resizeTo={containerRef}
          backgroundAlpha={0}
          antialias={false}
          autoDensity
          resolution={resolution}
        >
          <pixiContainer
            x={view.offsetX}
            y={view.offsetY}
            scale={view.zoom}
          >
            {texture !== Texture.EMPTY && (
              <pixiSprite
                texture={texture}
                scale={atlas ? 1 / atlas.meta.cell : 1}
              />
            )}
            <pixiGraphics draw={drawHover} />
            <pixiGraphics draw={drawPlayer} />
          </pixiContainer>
        </Application>
      ) : (
        <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
          No minimap yet.
        </div>
      )}
    </div>
  )
}
