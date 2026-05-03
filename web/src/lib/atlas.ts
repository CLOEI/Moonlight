export type AtlasMeta = {
  cell: number
  cols: number
  rows: number
  tiles: Record<string, [number, number]>
}

export type AtlasBundle = {
  meta: AtlasMeta
  image: HTMLImageElement
}

let atlasPromise: Promise<AtlasBundle | null> | null = null
let loaded: AtlasBundle | null = null

function loadOnce(): Promise<AtlasBundle | null> {
  if (atlasPromise) return atlasPromise
  atlasPromise = (async () => {
    try {
      const res = await fetch("/tiles/atlas.json", { cache: "force-cache" })
      if (!res.ok) return null
      const meta = (await res.json()) as AtlasMeta
      const image = await new Promise<HTMLImageElement>((resolve, reject) => {
        const img = new Image()
        img.onload = () => resolve(img)
        img.onerror = reject
        img.src = "/tiles/atlas.png"
      })
      return { meta, image }
    } catch {
      return null
    }
  })()
  return atlasPromise
}

if (typeof window !== "undefined") {
  loadOnce().then((bundle) => {
    loaded = bundle
  })
}

export function getAtlas(): Promise<AtlasBundle | null> {
  return loadOnce()
}

export function peekAtlas(): AtlasBundle | null {
  return loaded
}
