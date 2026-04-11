import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { Application, extend } from "@pixi/react"
import { Container, Graphics } from "pixi.js"

import {
  blendColor,
  fitMinimapView,
  minimapColor,
  tileAtCanvasPoint,
  type MinimapHoverTile,
  type MinimapView,
} from "@/lib/dashboard"
import type { MinimapSnapshot, PlayerPosition } from "@/lib/types"

extend({ Container, Graphics })

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

export function MinimapPanel({ minimap, playerPosition, onHoverChange }: Props) {
  const containerRef = useRef<HTMLDivElement | null>(null)
  const [size, setSize] = useState({ width: 0, height: 0 })
  const [view, setView] = useState<MinimapView>(INITIAL_VIEW)

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

  const drawTiles = useCallback(
    (graphics: Graphics) => {
      graphics.clear()
      if (!minimap) {
        return
      }

      for (let y = 0; y < minimap.height; y += 1) {
        for (let x = 0; x < minimap.width; x += 1) {
          const tileIndex = y * minimap.width + x
          let color = minimap.background_tiles[tileIndex]
            ? minimapColor(minimap.background_tiles[tileIndex], "background")
            : 0x87ceeb

          if (minimap.foreground_tiles[tileIndex]) {
            color = minimapColor(minimap.foreground_tiles[tileIndex], "foreground")
          }
          if (minimap.water_tiles[tileIndex]) {
            color = blendColor(
              color,
              minimapColor(minimap.water_tiles[tileIndex], "water"),
              0.55,
            )
          }
          if (minimap.wiring_tiles[tileIndex]) {
            color = blendColor(
              color,
              minimapColor(minimap.wiring_tiles[tileIndex], "wiring"),
              0.45,
            )
          }

          graphics.setFillStyle({ color })
          graphics.rect(x, minimap.height - y - 1, 1, 1)
          graphics.fill()
        }
      }
    },
    [minimap],
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

  const handleWheel = (event: React.WheelEvent<HTMLDivElement>) => {
    if (!minimap || !containerRef.current) {
      return
    }
    event.preventDefault()
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
  }

  return (
    <div
      ref={containerRef}
      className="relative h-80 overflow-hidden rounded-2xl border border-white/10 bg-[#081018]"
      onPointerDown={(event) => {
        pointerOrigin.current = {
          x: event.clientX,
          y: event.clientY,
          offsetX: view.offsetX,
          offsetY: view.offsetY,
        }
      }}
      onPointerUp={() => {
        pointerOrigin.current = null
      }}
      onPointerLeave={() => {
        pointerOrigin.current = null
        updateHover(null)
      }}
      onPointerMove={handlePointerMove}
      onWheel={handleWheel}
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
            <pixiGraphics draw={drawTiles} />
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
