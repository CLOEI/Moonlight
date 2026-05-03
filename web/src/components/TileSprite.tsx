import { useEffect, useState, type ReactNode } from "react"
import { getAtlas, peekAtlas, type AtlasBundle } from "@/lib/atlas"

function useAtlas(): AtlasBundle | null {
  const [bundle, setBundle] = useState<AtlasBundle | null>(() => peekAtlas())
  useEffect(() => {
    if (bundle) return
    let live = true
    getAtlas().then((b) => {
      if (live) setBundle(b)
    })
    return () => {
      live = false
    }
  }, [bundle])
  return bundle
}

type Props = {
  blockId: number
  size?: number
  fallback?: ReactNode
  className?: string
}

export function TileSprite({ blockId, size = 32, fallback, className = "" }: Props) {
  const bundle = useAtlas()
  const coords = bundle?.meta.tiles[String(blockId)]

  if (!bundle || !coords) {
    return (
      <div
        className={`rounded bg-secondary border border-white/10 flex items-center justify-center shrink-0 ${className}`}
        style={{ width: size, height: size }}
      >
        {fallback}
      </div>
    )
  }

  const cell = bundle.meta.cell
  const scale = size / cell
  const [sx, sy] = coords
  return (
    <div
      className={`rounded border border-white/10 shrink-0 overflow-hidden ${className}`}
      style={{ width: size, height: size }}
    >
      <div
        style={{
          width: cell,
          height: cell,
          backgroundImage: "url(/tiles/atlas.png)",
          backgroundPosition: `-${sx}px -${sy}px`,
          backgroundRepeat: "no-repeat",
          imageRendering: "pixelated",
          transform: `scale(${scale})`,
          transformOrigin: "top left",
        }}
      />
    </div>
  )
}
